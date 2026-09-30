// trex GPU chunked context-mixing coder.
//
// One thread codes one chunk of the input independently, so thousands of
// chunks run concurrently across the SMs - the GPU analogue of the CPU's
// Flynnel `compress_chunks`. Each thread runs a context-mixing coder over its
// chunk and writes the chunk's code length in bits; the host sums them. The
// design tension is memory vs ratio: a full-size table per thread will not fit
// thousands of threads, and tiny chunks never warm up, so each thread carries
// SMALL direct-mapped tables and chunks are sized to a few KB.
//
// Ported from the CPU logistic-mix coder: literal order-k byte models AND
// case-folded (orbit) order-k models with direct-mapped count tables, an LZP
// match model, blended by an online integer logistic mixer, arithmetic-coded
// per bit. Embeddings / APM are layered on next.
//
// Compiled to PTX for a Turing (sm_75) floor and JIT-compiled up to the running
// device by the driver, so deployment needs only the driver.

__device__ __forceinline__ int squash_i(int x) {
    if (x <= -2047) return 1;
    if (x >= 2047) return 4095;
    double v = 4096.0 / (1.0 + exp(-((double)x) / 256.0));
    int p = (int)(v + 0.5);
    return p < 1 ? 1 : (p > 4095 ? 4095 : p);
}
__device__ __forceinline__ int stretch_i(int p) {
    if (p < 1) p = 1;
    if (p > 4095) p = 4095;
    double s = 256.0 * log((double)p / (double)(4096 - p));
    int v = (int)(s > 0 ? s + 0.5 : s - 0.5);
    return v < -2047 ? -2047 : (v > 2047 ? 2047 : v);
}

// Natural-unit logistic for the learned-embedding gradient (log-odds -> prob).
__device__ __forceinline__ float squash_f(float x) { return 1.0f / (1.0f + expf(-x)); }

#define NCTX 11                // the CPU coder's spec list, exactly:
                               // literal 0,1,2,3,4,6,8 + case-fold 4 + shape 3,6 + word-stem
#define NEMB 2                 // learned E8 embeddings at orders 5 and 8
#define NIN (NCTX + 1 + NEMB + 1) // contexts + match + embeddings + neural trace
#define EMB_BITS_G 14          // per-thread embedding table (smaller than CPU's)
#define EMB_SIZE (1 << EMB_BITS_G)
#define EMB_MASK (EMB_SIZE - 1)
#define EMB_DIM 16
#define EMB_FRAC 16777216.0    // 2^24 fixed-point store
#define EMB_LR 0.03f
// The per-thread context tables are sized at launch (ctx_bits kernel arg):
// bigger chunks need bigger tables or evictions destroy the high-order counts,
// mirroring the CPU coder's input-proportional table sizing.
#define MATCH_H 6              // match-model context length in bytes
#define MATCH_BITS 14
#define MATCH_SIZE (1 << MATCH_BITS)
#define MATCH_MASK (MATCH_SIZE - 1)
#define MIXK 0x9E3779B97F4A7C15ULL
#define FNV_OFFSET 14695981039346656037ULL
#define FNV_PRIME 1099511628211ULL

struct Slot {
    unsigned short check;
    unsigned short n0;
    unsigned short n1;
};

// Case-fold one byte for the orbit models: upper-case to lower-case.
__device__ __forceinline__ unsigned char fold_byte(unsigned char b) {
    return (b >= 'A' && b <= 'Z') ? (unsigned char)(b + 32) : b;
}

// The character shape class of a byte (vowel / consonant / digit / other),
// matching the CPU's orbit::shape_char.
__device__ __forceinline__ unsigned char shape_byte(unsigned char b) {
    if (b >= '0' && b <= '9') return (unsigned char)'D';
    if (b == 'a' || b == 'e' || b == 'i' || b == 'o' || b == 'u' ||
        b == 'A' || b == 'E' || b == 'I' || b == 'O' || b == 'U')
        return (unsigned char)'V';
    if ((b >= 'a' && b <= 'z') || (b >= 'A' && b <= 'Z')) return (unsigned char)'C';
    return (unsigned char)'.';
}
__device__ __forceinline__ int is_alnum_b(unsigned char b) {
    return (b >= '0' && b <= '9') || (b >= 'a' && b <= 'z') || (b >= 'A' && b <= 'Z');
}

// Model kinds, matching the CPU spec list: 0 = literal last-k bytes, 1 =
// case-folded, 2 = shape-class, 3 = word-stem (the lowercased alnum run
// ending at t, up to 32 bytes - the morphological orbit).
__constant__ int MODEL_ORDER_C[NCTX] = {0, 1, 2, 3, 4, 6, 8, 4, 3, 6, 0};
__constant__ int MODEL_KIND_C[NCTX] = {0, 0, 0, 0, 0, 0, 0, 1, 2, 2, 3};

// The context hash for model k at position t. Windows may cross the chunk
// boundary (clamped at 0, not at the chunk start): the overlap warmup already
// exposes the preceding bytes, so the main loop sees the same context stream.
__device__ unsigned long long model_hash(const unsigned char* __restrict__ input, int t, int k) {
    unsigned long long h = FNV_OFFSET ^ (unsigned long long)(k + 1);
    int kind = MODEL_KIND_C[k];
    if (kind == 3) {
        int s = t;
        while (s > 0 && is_alnum_b(input[s - 1]) && t - s < 32) s--;
        for (int j = s; j < t; ++j) h = (h ^ fold_byte(input[j])) * FNV_PRIME;
    } else {
        int lo = t - MODEL_ORDER_C[k];
        if (lo < 0) lo = 0;
        for (int j = lo; j < t; ++j) {
            unsigned char b = input[j];
            if (kind == 1) b = fold_byte(b);
            else if (kind == 2) b = shape_byte(b);
            h = (h ^ b) * FNV_PRIME;
        }
    }
    return h;
}

extern "C" __global__ void trex_compress(
    const unsigned char* __restrict__ input,   // [total_len]
    const int* __restrict__ chunk_start,        // [nc]
    const int* __restrict__ chunk_end,          // [nc]
    int nc,
    const Slot* __restrict__ prior,             // shared baked prior [NCTX * (1<<prior_bits)]
    int prior_bits,                             // log2 of the prior table size per model
    int prior_predict,                          // 1: a slot miss also predicts from the prior
    int overlap,                                // warm tables on input[start-overlap..start]
    int ctx_bits,                               // log2 slots per model per thread
    Slot* tables,                               // [nc * NCTX * (1<<ctx_bits)]
    unsigned int* mtables,                      // [nc * MATCH_SIZE], pos+1 (0=empty)
    int* emb_g,                                 // [nc * NEMB * EMB_SIZE * EMB_DIM]
    int* wnode_g,                               // [nc * NEMB * 256 * EMB_DIM]
    int* weights_g,                             // [nc * 256 * NIN] per-selector mixer
    double* __restrict__ out_bits)              // [nc]
{
    int c = blockIdx.x * blockDim.x + threadIdx.x;
    if (c >= nc) return;
    int start = chunk_start[c];
    int end = chunk_end[c];
    size_t ctx_size = (size_t)1 << ctx_bits;
    unsigned int ctx_mask = (unsigned int)(ctx_size - 1);
    Slot* tab = tables + (size_t)c * NCTX * ctx_size;
    unsigned int* mtab = mtables + (size_t)c * MATCH_SIZE;
    int* emb_t = emb_g + (size_t)c * NEMB * EMB_SIZE * EMB_DIM;
    int* wnode_t = wnode_g + (size_t)c * NEMB * 256 * EMB_DIM;
    const int emb_order[NEMB] = {5, 8};
    // Seed the embedding vectors with small pseudo-random values (wnode is zeroed
    // by the host), so the learned metric can separate contexts.
    {
        unsigned long long s = 0x123456789abcdef0ULL ^ (((unsigned long long)c + 1) * MIXK);
        int total = NEMB * EMB_SIZE * EMB_DIM;
        for (int i = 0; i < total; ++i) {
            s = s * 6364136223846793005ULL + 1442695040888963407ULL;
            emb_t[i] = (int)((((double)(s >> 40) / 16777216.0) - 0.5) * 0.2 * EMB_FRAC);
        }
    }

    // Neural trace predictor: multi-timescale bit traces per bit-tree node with
    // learned non-negative normalized weights, matching the CPU coder. Small
    // per-thread state (256 x 3 x 2 floats), fast-adapting - no training-data
    // hunger, unlike the embeddings.
    float nt_trace[256][3];
    float nt_w[256][3];
    for (int i = 0; i < 256; ++i) {
        nt_trace[i][0] = 0.0f; nt_trace[i][1] = 0.0f; nt_trace[i][2] = 0.0f;
        nt_w[i][0] = 0.34f; nt_w[i][1] = 0.33f; nt_w[i][2] = 0.33f;
    }
    const float NT_DECAY[3] = {0.7f, 0.9f, 0.97f};

    // Prior lookup: a slot new to this key seeds from the shared baked prior
    // (built once on the host from a fixed English corpus with THIS kernel's
    // hashing, so the keys align), the device analogue of the CPU coder's
    // baked_for_mi. `prior_bits == 0` means no prior (seed uniform).
    size_t psize = (size_t)1 << prior_bits;
    unsigned int pmask = (unsigned int)(psize - 1);

    // Per-selector mixer weights (one NIN-vector per previous byte), so the
    // mixer specialises per local regime. Context models start at 0.3; the match
    // and embedding inputs start ignored (0) and are raised only if they help.
    int* wt = weights_g + (size_t)c * 256 * NIN;
    for (int s = 0; s < 256; ++s)
        for (int k = 0; k < NIN; ++k)
            wt[s * NIN + k] = (k < NCTX) ? (int)(0.3 * 65536.0) : 0;

    int mp = -1, ml = 0; // match pointer and length

    // Overlapping context: warm the tables on the real bytes immediately before
    // this chunk (seeding new slots from the prior), so the chunk begins with
    // genuine local context, not just generic English. This recovers most of
    // the whole-stream adaptation the sequential CPU coder has. No bits are
    // charged for the warmup; only the count tables move.
    int wstart = start - overlap;
    if (wstart < 0) wstart = 0;
    for (int t = wstart; t < start; ++t) {
        unsigned char wb = input[t];
        unsigned long long wbase[NCTX];
        for (int k = 0; k < NCTX; ++k) wbase[k] = model_hash(input, t, k);
        unsigned long long wnode = 1;
        for (int bit = 7; bit >= 0; --bit) {
            int wa = (wb >> bit) & 1;
            for (int k = 0; k < NCTX; ++k) {
                unsigned long long key = wbase[k] * MIXK + wnode;
                unsigned int idx = (unsigned int)key & ctx_mask;
                unsigned short chk = (unsigned short)(key >> ctx_bits);
                Slot* s = &tab[(size_t)k * ctx_size + idx];
                if (s->check != chk) {
                    s->check = chk;
                    s->n0 = 0;
                    s->n1 = 0;
                    if (prior_bits) {
                        unsigned int pidx = (unsigned int)key & pmask;
                        unsigned short pchk = (unsigned short)(key >> prior_bits);
                        Slot ps = prior[(size_t)k * psize + pidx];
                        if (ps.check == pchk) { s->n0 = ps.n0; s->n1 = ps.n1; }
                    }
                }
                if (wa) s->n1++; else s->n0++;
                if (s->n0 + s->n1 > 1024) { s->n0 = (s->n0 + 1) >> 1; s->n1 = (s->n1 + 1) >> 1; }
            }
            wnode = (wnode << 1) | (unsigned long long)wa;
        }
    }

    double bits = 0.0;

    for (int t = start; t < end; ++t) {
        unsigned char byte = input[t];
        // Per-model context hashes (literal / fold / shape / stem), windows
        // crossing the chunk boundary like the warmup's.
        unsigned long long base[NCTX];
        for (int k = 0; k < NCTX; ++k) base[k] = model_hash(input, t, k);
        // Embedding context indices (order-5 and order-8 byte contexts).
        unsigned int ec[NEMB];
        for (int oi = 0; oi < NEMB; ++oi) {
            int o = emb_order[oi];
            unsigned long long h = FNV_OFFSET ^ (0x1dULL ^ (unsigned long long)o);
            int lo = t - o;
            if (lo < start) lo = start;
            for (int j = lo; j < t; ++j) h = (h ^ input[j]) * FNV_PRIME;
            ec[oi] = (unsigned int)h & EMB_MASK;
        }
        // Match model: hash the last MATCH_H bytes; acquire a match if idle.
        unsigned int mhi = 0;
        bool have_mh = (t - start) >= MATCH_H;
        if (have_mh) {
            unsigned long long mh = FNV_OFFSET;
            for (int j = t - MATCH_H; j < t; ++j) mh = (mh ^ input[j]) * FNV_PRIME;
            mhi = (unsigned int)mh & MATCH_MASK;
            if (ml == 0) {
                unsigned int p = mtab[mhi];
                if (p != 0 && (int)(p - 1) < t) { mp = (int)(p - 1); ml = MATCH_H; }
            }
        }
        int predicted = (ml > 0 && mp >= 0 && mp < t) ? input[mp] : -1;
        bool matched = predicted >= 0;
        int sel = (t > 0) ? input[t - 1] : 0; // mixer selector: the previous byte
        int* w = wt + sel * NIN;

        unsigned long long node = 1;
        for (int bit = 7; bit >= 0; --bit) {
            int actual = (byte >> bit) & 1;
            int nn = (int)(node & 0xFF);
            int st[NIN];
            for (int k = 0; k < NCTX; ++k) {
                unsigned long long key = base[k] * MIXK + node;
                unsigned int idx = (unsigned int)key & ctx_mask;
                unsigned short chk = (unsigned short)(key >> ctx_bits);
                Slot s = tab[(size_t)k * ctx_size + idx];
                int n0 = 0, n1 = 0;
                if (s.check == chk) {
                    n0 = s.n0;
                    n1 = s.n1;
                } else if (prior_predict && prior_bits) {
                    // A key new to its slot predicts from the prior, as the CPU
                    // coder's miss does, rather than from uniform.
                    unsigned int pidx = (unsigned int)key & pmask;
                    unsigned short pchk = (unsigned short)(key >> prior_bits);
                    Slot ps = prior[(size_t)k * psize + pidx];
                    if (ps.check == pchk) { n0 = ps.n0; n1 = ps.n1; }
                }
                int p12 = (int)(((unsigned long long)(2 * n1 + 1) * 4096) / (unsigned long long)(2 * (n0 + n1) + 2));
                st[k] = stretch_i(p12);
            }
            // Match input: while the predicted byte holds, its bit with a
            // confidence rising in the match length.
            if (matched) {
                int conf = ml < 28 ? ml * 82 : 28 * 82;
                if (conf > 2047) conf = 2047;
                st[NCTX] = ((predicted >> bit) & 1) ? conf : -conf;
            } else {
                st[NCTX] = 0;
            }
            // Learned embedding inputs: the bilinear dot of the context vector
            // and the bit-node vector, per order.
            float emb_logit[NEMB];
            for (int oi = 0; oi < NEMB; ++oi) {
                int* e = emb_t + ((size_t)oi * EMB_SIZE + ec[oi]) * EMB_DIM;
                int* w = wnode_t + ((size_t)oi * 256 + nn) * EMB_DIM;
                long long ed = 0;
                for (int k = 0; k < EMB_DIM; ++k) ed += (long long)e[k] * (long long)w[k];
                float logit = (float)((double)ed / (EMB_FRAC * EMB_FRAC));
                if (logit < -12.0f) logit = -12.0f;
                if (logit > 12.0f) logit = 12.0f;
                emb_logit[oi] = logit;
                int si = (int)(logit * 256.0f);
                if (si < -2047) si = -2047;
                if (si > 2047) si = 2047;
                st[NCTX + 1 + oi] = si;
            }
            // Neural trace input (keyed by the bit-tree node).
            float p_nt = nt_w[nn][0] * nt_trace[nn][0] + nt_w[nn][1] * nt_trace[nn][1]
                + nt_w[nn][2] * nt_trace[nn][2];
            if (p_nt < 1e-3f) p_nt = 1e-3f;
            if (p_nt > 0.999f) p_nt = 0.999f;
            st[NIN - 1] = stretch_i((int)(p_nt * 4096.0f));
            long long dot = 0;
            for (int k = 0; k < NIN; ++k) dot += (long long)w[k] * (long long)st[k];
            int x = (int)(dot >> 16);
            if (x < -2047) x = -2047;
            if (x > 2047) x = 2047;
            int p = squash_i(x);
            double pbit = actual ? (double)p / 4096.0 : 1.0 - (double)p / 4096.0;
            if (pbit < 1e-6) pbit = 1e-6;
            bits += -log2(pbit);
            int err = (actual << 12) - p;
            for (int k = 0; k < NIN; ++k)
                w[k] += (int)(((long long)err * (long long)st[k] * 82) >> 16);
            for (int k = 0; k < NCTX; ++k) {
                unsigned long long key = base[k] * MIXK + node;
                unsigned int idx = (unsigned int)key & ctx_mask;
                unsigned short chk = (unsigned short)(key >> ctx_bits);
                Slot* s = &tab[(size_t)k * ctx_size + idx];
                if (s->check != chk) {
                    s->check = chk;
                    s->n0 = 0;
                    s->n1 = 0;
                    if (prior_bits) {
                        unsigned int pidx = (unsigned int)key & pmask;
                        unsigned short pchk = (unsigned short)(key >> prior_bits);
                        Slot ps = prior[(size_t)k * psize + pidx];
                        if (ps.check == pchk) { s->n0 = ps.n0; s->n1 = ps.n1; }
                    }
                }
                if (actual) s->n1++; else s->n0++;
                if (s->n0 + s->n1 > 1024) { s->n0 = (s->n0 + 1) >> 1; s->n1 = (s->n1 + 1) >> 1; }
            }
            // Embedding gradient: one online step on each order's vector and the
            // bit-node vector (fixed-point store, f32 gradient).
            for (int oi = 0; oi < NEMB; ++oi) {
                float el = (float)actual - squash_f(emb_logit[oi]);
                int* e = emb_t + ((size_t)oi * EMB_SIZE + ec[oi]) * EMB_DIM;
                int* w = wnode_t + ((size_t)oi * 256 + nn) * EMB_DIM;
                for (int k = 0; k < EMB_DIM; ++k) {
                    int ev = e[k];
                    int wv = w[k];
                    e[k] = ev + (int)(EMB_LR * el * (float)wv);
                    w[k] = wv + (int)(EMB_LR * el * (float)ev);
                }
            }
            // Neural trace self-update: a gradient step on its OWN error with
            // normalized non-negative weights, then observe the bit.
            {
                float nt_err = (float)actual - p_nt;
                float wsum = 0.0f;
                for (int d = 0; d < 3; ++d) {
                    nt_w[nn][d] += 0.01f * nt_err * nt_trace[nn][d];
                    if (nt_w[nn][d] < 0.01f) nt_w[nn][d] = 0.01f;
                    wsum += nt_w[nn][d];
                }
                for (int d = 0; d < 3; ++d) {
                    nt_w[nn][d] /= wsum;
                    nt_trace[nn][d] = NT_DECAY[d] * nt_trace[nn][d] + (1.0f - NT_DECAY[d]) * (float)actual;
                }
            }
            if (matched && ((predicted >> bit) & 1) != actual) matched = false;
            node = (node << 1) | (unsigned long long)actual;
        }
        // Match update: extend on a full-byte hit, else drop; record position.
        if (predicted >= 0) {
            if (predicted == byte) { ml++; mp++; } else ml = 0;
        }
        if (have_mh) mtab[mhi] = (unsigned int)(t + 1);
    }
    out_bits[c] = bits;
}
