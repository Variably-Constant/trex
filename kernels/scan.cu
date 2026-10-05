// trex GPU SIMT scan kernel.
//
// One thread per significant-token anchor runs a bit-parallel NFA
// simulation over the token-kind stream. The active set of NFA states is
// a 64-bit mask indexed by program counter; an active atom state whose
// required kind matches the current token activates the epsilon-closure
// of its successor. The accept state being active means a match ends at
// the current position, and the largest such position is the longest
// match anchored at this thread's start. The host then selects the
// leftmost, non-overlapping matches, exactly as the CPU engine does.
//
// The kernel handles the regular, alternation-free, capture-free subset
// of trex patterns (typed-kind atoms and `.`, with concatenation and
// quantifiers); the host routes every other pattern to the CPU engine,
// so the GPU and CPU results are identical on what the GPU accepts.
//
// Compiled to PTX for a Turing (sm_75) floor and JIT-compiled up to the
// running device by the driver, so deployment needs only the driver.

extern "C" __global__ void trex_scan(
    const unsigned int* __restrict__ token_kind,      // [n]
    int n,
    const unsigned int* __restrict__ atom_kind,       // [nstates] required kind, or ANY sentinel
    const unsigned long long* __restrict__ next_closure, // [nstates] closure(pc+1)
    const unsigned int* __restrict__ is_atom,         // [nstates] 1 if pc consumes a token
    unsigned long long start_closure,
    unsigned long long match_mask,
    int nstates,
    int* __restrict__ result_end)                     // [n] longest match end, or -1
{
    int a = blockIdx.x * blockDim.x + threadIdx.x;
    if (a >= n) return;

    const unsigned int ANY = 0xFFFFFFFFu;
    unsigned long long active = start_closure;
    int best = -1;

    // The NFA tables are read through `const ... __restrict__` parameters,
    // so the compiler routes them through the read-only data cache; they
    // are tiny (at most 64 states) and stay resident there. Staging them in
    // shared memory was measured to give no end-to-end gain: the kernel is
    // ~0.03% of the device call, which is dominated by the host-side lex and
    // the host/device copies, not these reads.
    for (int k = a; k < n; ++k) {
        if (active & match_mask) {
            best = k; // a match consuming tokens [a, k) is reachable
        }
        unsigned long long next = 0ULL;
        unsigned long long m = active;
        unsigned int tk = token_kind[k];
        while (m) {
            int pc = __ffsll((long long)m) - 1; // index of lowest set bit
            m &= m - 1ULL;
            if (is_atom[pc]) {
                unsigned int ak = atom_kind[pc];
                if (ak == ANY || ak == tk) {
                    next |= next_closure[pc];
                }
            }
        }
        if (next == 0ULL) {
            active = 0ULL;
            break;
        }
        active = next;
    }
    if (active & match_mask) {
        best = n; // a match consuming tokens [a, n) is reachable
    }
    result_end[a] = best;
}

// `trex_scan` over per-token properties as well as kinds. A token passes an
// atom state only when its kind fits; its magnitude x100 is in
// [mag_lo, mag_hi] for that state when the pattern reads magnitudes; every
// byte class in need_mask holds for all its bytes; at a literal its orbit
// class equals lit_class; at a register reference its orbit class equals the
// class of the token `ref_back` places earlier; and, when the pattern reads
// the spectral field, its pooled entropy x100 is in [ent_lo, ent_hi], its
// dominant period satisfies period_need (0 no test, 1 any nonzero, 2 equal to
// period_val), its texture class equals texture_need unless that is ANY, and
// a change-point is inside it when onset_need is set. The spectral word per
// token packs the period in its low sixteen bits, the texture class in the
// next four, and the onset flag in bit 20. A state with no magnitude or
// entropy test carries negative and positive infinity, no byte test 0, no
// literal ANY, no reference 0. The host computes every property as the CPU
// engine compares it. A property the pattern does not read arrives as a
// one-entry buffer that is never indexed.
extern "C" __global__ void trex_scan_props(
    const unsigned int* __restrict__ token_kind,      // [n]
    int n,
    const float* __restrict__ token_mag100,           // [n] magnitude x100, or [1] unread
    int reads_mag,
    const unsigned int* __restrict__ token_class,     // [n] orbit class id, or [1] unread
    const unsigned int* __restrict__ token_bytes,     // [n] byte-class mask, or [1] unread
    const float* __restrict__ token_ent100,           // [n] pooled entropy x100, or [1] unread
    const unsigned int* __restrict__ token_spec,      // [n] period | texture << 16 | onset << 20, or [1] unread
    int reads_spec,
    const unsigned int* __restrict__ atom_kind,       // [nstates] required kind, or ANY sentinel
    const float* __restrict__ mag_lo,                 // [nstates] least magnitude x100
    const float* __restrict__ mag_hi,                 // [nstates] greatest magnitude x100
    const int* __restrict__ ref_back,                 // [nstates] tokens back to the bound token, or 0
    const unsigned int* __restrict__ need_mask,       // [nstates] byte classes every byte must hold, or 0
    const unsigned int* __restrict__ lit_class,       // [nstates] the literal's orbit class id, or ANY
    const float* __restrict__ ent_lo,                 // [nstates] least entropy x100
    const float* __restrict__ ent_hi,                 // [nstates] greatest entropy x100
    const unsigned int* __restrict__ period_need,     // [nstates] 0 none, 1 any, 2 equal
    const unsigned int* __restrict__ period_val,      // [nstates] the period an equal test needs
    const unsigned int* __restrict__ texture_need,    // [nstates] texture class id, or ANY
    const unsigned int* __restrict__ onset_need,      // [nstates] 1 when a change-point must be inside
    const unsigned long long* __restrict__ next_closure, // [nstates] closure(pc+1)
    const unsigned int* __restrict__ is_atom,         // [nstates] 1 if pc consumes a token
    unsigned long long start_closure,
    unsigned long long match_mask,
    int nstates,
    int* __restrict__ result_end)                     // [n] longest match end, or -1
{
    int a = blockIdx.x * blockDim.x + threadIdx.x;
    if (a >= n) return;

    const unsigned int ANY = 0xFFFFFFFFu;
    unsigned long long active = start_closure;
    int best = -1;

    for (int k = a; k < n; ++k) {
        if (active & match_mask) {
            best = k; // a match consuming tokens [a, k) is reachable
        }
        unsigned long long next = 0ULL;
        unsigned long long m = active;
        unsigned int tk = token_kind[k];
        float tm = reads_mag ? token_mag100[k] : 0.0f;
        float te = reads_spec ? token_ent100[k] : 0.0f;
        unsigned int ts = reads_spec ? token_spec[k] : 0u;
        while (m) {
            int pc = __ffsll((long long)m) - 1; // index of lowest set bit
            m &= m - 1ULL;
            if (!is_atom[pc]) continue;
            unsigned int ak = atom_kind[pc];
            if (ak != ANY && ak != tk) continue;
            if (reads_mag && (tm < mag_lo[pc] || tm > mag_hi[pc])) continue;
            if (reads_spec) {
                if (te < ent_lo[pc] || te > ent_hi[pc]) continue;
                unsigned int pn = period_need[pc];
                unsigned int period = ts & 0xFFFFu;
                if (pn == 1u && period == 0u) continue;
                if (pn == 2u && period != period_val[pc]) continue;
                unsigned int tn = texture_need[pc];
                if (tn != ANY && ((ts >> 16) & 0xFu) != tn) continue;
                if (onset_need[pc] && ((ts >> 20) & 1u) == 0u) continue;
            }
            unsigned int need = need_mask[pc];
            if (need != 0 && (token_bytes[k] & need) != need) continue;
            unsigned int lc = lit_class[pc];
            if (lc != ANY && token_class[k] != lc) continue;
            int rb = ref_back[pc];
            // The bound token is inside this anchor's attempt; the bound
            // check keeps a wrong offset from reading before it.
            if (rb > 0 && (k - rb < a || token_class[k] != token_class[k - rb])) continue;
            next |= next_closure[pc];
        }
        if (next == 0ULL) {
            active = 0ULL;
            break;
        }
        active = next;
    }
    if (active & match_mask) {
        best = n; // a match consuming tokens [a, n) is reachable
    }
    result_end[a] = best;
}
