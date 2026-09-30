//! The device coder behind `trex compress --gpu` and `--hybrid`: the input
//! coded in KB-scale chunks, one per CUDA thread, against the baked byte-ngram
//! prior projected into the device's tables.
//!
//! Compiled with the `compress` feature; `gpu` re-exports every public item
//! here, so each keeps its `trex::gpu::` path. Without the `gpu` feature every
//! entry point returns `None` and the caller codes on the CPU.

/// Compress `input` on the GPU with the chunked context-mixing coder, returning
/// the total code length in bits, or `None` when the `gpu` feature is off or no
/// device is present. The GPU coder slices the input into KB-scale chunks and
/// codes them concurrently across the SMs - the device analogue of the CPU's
/// Flynnel `compress_chunks`. On `None` the caller uses the CPU coder.
#[must_use]
#[cfg_attr(not(feature = "gpu"), allow(unused_variables))]
pub fn compress_gpu(input: &[u8]) -> Option<f64> {
    #[cfg(feature = "gpu")]
    {
        cuda::compress(input)
    }
    #[cfg(not(feature = "gpu"))]
    {
        None
    }
}

/// Where a device compress spends itself, phase by phase.
///
/// Each upload and allocation is followed by a synchronize, so the clock sees
/// the copy land rather than the cost of queueing it. The prior is timed on its
/// own because it is the same table on every call, which makes it the part
/// residency would remove.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct CompressPhases {
    /// Chunks coded, one per device thread.
    pub chunks: usize,
    /// Bytes of prior sent to the device.
    pub prior_bytes: usize,
    /// Chunk bounds on the host and fetching the cached prior.
    pub host_prep_us: f64,
    /// Copying the input and the chunk bounds to the device.
    pub upload_input_us: f64,
    /// Allocating and zeroing the per-thread tables, match hashes,
    /// embeddings and mixer weights.
    pub alloc_us: f64,
    /// Copying the prior to the device.
    pub upload_prior_us: f64,
    /// The kernel, from launch to the synchronize after the download.
    pub kernel_and_download_us: f64,
}

/// How much of one device model a projected prior kept.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ProjectionStats {
    /// The device model, an index into the kernel's model list.
    pub model: usize,
    /// Distinct bit-tree nodes the blob gives this model.
    pub nodes: usize,
    /// Nodes that won a slot.
    pub placed: usize,
    /// Share of the model's total count carried by the nodes that won.
    pub mass_kept: f64,
}

/// Project a serialized byte-ngram prior - the format of the compiled-in CPU
/// prior - into a device prior table at `bits` per model, with each slot's
/// counts halved until they total at most `cap`.
///
/// Returns `None` in a build without the `gpu` feature, where the device
/// table's hashing is not compiled.
#[must_use]
#[cfg_attr(not(feature = "gpu"), allow(unused_variables))]
pub fn project_device_prior(blob: &[u8], bits: usize, cap: u64) -> Option<(Vec<u16>, Vec<ProjectionStats>)> {
    #[cfg(feature = "gpu")]
    {
        Some(cuda::project_prior(blob, bits, cap))
    }
    #[cfg(not(feature = "gpu"))]
    {
        None
    }
}

/// [`compress_gpu`] reading `prior`, a table from [`project_device_prior`] at
/// `bits`. Zero bits codes with no prior; `prior` must then be non-empty and
/// is never read. With `predict`, a context model's slot miss predicts from the
/// prior, as the CPU coder's does; without it the miss predicts uniform and the
/// prior only seeds the slot's counts.
#[must_use]
#[cfg_attr(not(feature = "gpu"), allow(unused_variables))]
pub fn compress_gpu_with_prior(input: &[u8], prior: &[u16], bits: usize, predict: bool) -> Option<f64> {
    #[cfg(feature = "gpu")]
    {
        cuda::compress_with(input, prior, bits, predict)
    }
    #[cfg(not(feature = "gpu"))]
    {
        None
    }
}

/// A device prior table held on the device, so each compress against it sends
/// only its input.
pub struct GpuPrior {
    #[cfg(feature = "gpu")]
    held: cuda::HeldPrior,
}

impl GpuPrior {
    /// Hold `table`, a device prior at `bits` from [`project_device_prior`];
    /// zero bits takes a non-empty table the coder never reads. `predict` as in
    /// [`compress_gpu_with_prior`]. `None` when the `gpu` feature is off or no
    /// usable device is present.
    #[must_use]
    #[cfg_attr(not(feature = "gpu"), allow(unused_variables))]
    pub fn upload(table: &[u16], bits: usize, predict: bool) -> Option<Self> {
        #[cfg(feature = "gpu")]
        {
            cuda::hold_prior(table, bits, predict).map(|held| Self { held })
        }
        #[cfg(not(feature = "gpu"))]
        {
            None
        }
    }

    /// Hold the prior [`compress_gpu`] codes against, read as it reads it, or
    /// `None` when the `gpu` feature is off or no usable device is present.
    #[must_use]
    pub fn default_prior() -> Option<Self> {
        #[cfg(feature = "gpu")]
        {
            cuda::hold_default_prior().map(|held| Self { held })
        }
        #[cfg(not(feature = "gpu"))]
        {
            None
        }
    }

    /// Code `input` against the held prior: the bits [`compress_gpu_with_prior`]
    /// returns for the same table, or `None` when the device call fails.
    #[must_use]
    #[cfg_attr(not(feature = "gpu"), allow(unused_variables))]
    pub fn compress(&self, input: &[u8]) -> Option<f64> {
        #[cfg(feature = "gpu")]
        {
            cuda::compress_held(input, &self.held)
        }
        #[cfg(not(feature = "gpu"))]
        {
            None
        }
    }
}

/// [`compress_gpu`] with a clock between its phases. Returns the same code
/// length in bits; `None` without the `gpu` feature or a usable device.
#[must_use]
#[cfg_attr(not(feature = "gpu"), allow(unused_variables))]
pub fn compress_gpu_phases(input: &[u8]) -> Option<(f64, CompressPhases)> {
    #[cfg(feature = "gpu")]
    {
        cuda::compress_phases(input)
    }
    #[cfg(not(feature = "gpu"))]
    {
        None
    }
}

#[cfg(feature = "gpu")]
#[allow(unsafe_code)]
mod cuda {
    use std::sync::{Arc, OnceLock};

    use cudarc::driver::{CudaContext, CudaFunction, LaunchConfig, PushKernelArg};
    use cudarc::nvrtc::Ptx;

    use crate::gpu::cuda::load_or_cpu;

    /// PTX compiled from `kernels/compress.cu` at build time (see `build.rs`).
    const COMPRESS_PTX: &str = include_str!(env!("TREX_COMPRESS_PTX"));
    /// Must match `kernels/compress.cu`: NCTX context models (literal + orbit)
    /// with 2^14-slot tables each, an LZP match hash of 2^14 positions, and
    /// KB-scale chunks.
    const NCTX: usize = 11;
    const MATCH_SIZE: usize = 1 << 14;
    const CHUNK: usize = 8192;
    // Learned-embedding sizes; must match kernels/compress.cu.
    const NEMB: usize = 2;
    const EMB_SIZE: usize = 1 << 14;
    const EMB_DIM: usize = 16;

    /// A fixed English corpus (Pride & Prejudice sample), compiled in. A
    /// projected device prior takes its word-stem model's counts from it, since
    /// no fixed-order row of the byte-ngram prior holds a stem context.
    const PRIOR_CORPUS: &[u8] = include_bytes!("../../_corpus/prior_en.txt");
    /// log2 of the default device prior's slots per model.
    const PRIOR_BITS: usize = 24;
    /// Most the two counts of a default device prior slot total.
    const PRIOR_CAP: u64 = 8;
    // Must match `kernels/compress.cu`.
    // Kinds match the kernel's MODEL_KIND_C: 0 = literal, 1 = case-fold,
    // 2 = shape-class, 3 = word-stem.
    const MODEL_ORDER: [usize; NCTX] = [0, 1, 2, 3, 4, 6, 8, 4, 3, 6, 0];
    const MODEL_KIND: [u8; NCTX] = [0, 0, 0, 0, 0, 0, 0, 1, 2, 2, 3];
    const MIXK: u64 = 0x9E37_79B9_7F4A_7C15;
    const FNV_OFFSET: u64 = 14695981039346656037;
    const FNV_PRIME: u64 = 1099511628211;

    fn fold_byte(b: u8) -> u8 {
        if b.is_ascii_uppercase() { b + 32 } else { b }
    }

    /// The shape class of a byte, matching the kernel's `shape_byte` (and the
    /// coder's `orbit::shape_char`).
    fn shape_byte(b: u8) -> u8 {
        if b.is_ascii_digit() {
            b'D'
        } else if matches!(b, b'a' | b'e' | b'i' | b'o' | b'u' | b'A' | b'E' | b'I' | b'O' | b'U') {
            b'V'
        } else if b.is_ascii_alphabetic() {
            b'C'
        } else {
            b'.'
        }
    }

    /// The context hash for model `k` at position `t` over `data` - it must stay
    /// byte-identical to the kernel's `model_hash`, or the prior keys never
    /// match the kernel keys (the FNV_OFFSET incident).
    fn model_hash(data: &[u8], t: usize, k: usize) -> u64 {
        let mut h = FNV_OFFSET ^ (k as u64 + 1);
        if MODEL_KIND[k] == 3 {
            let mut s = t;
            while s > 0 && data[s - 1].is_ascii_alphanumeric() && t - s < 32 {
                s -= 1;
            }
            for &raw in &data[s..t] {
                h = (h ^ u64::from(fold_byte(raw))).wrapping_mul(FNV_PRIME);
            }
        } else {
            let lo = t.saturating_sub(MODEL_ORDER[k]);
            for &raw in &data[lo..t] {
                let by = match MODEL_KIND[k] {
                    1 => fold_byte(raw),
                    2 => shape_byte(raw),
                    _ => raw,
                };
                h = (h ^ u64::from(by)).wrapping_mul(FNV_PRIME);
            }
        }
        h
    }

    /// A device prior counted from `data`, using the same FNV hashing and
    /// models as the kernel so the keys align. Flat NCTX * 2^bits slots of
    /// (check, n0, n1) as u16 triples, matching the kernel's `Slot`.
    fn corpus_prior(data: &[u8], bits: usize) -> Vec<u16> {
        let psize = 1usize << bits;
        let pmask = psize - 1;
        let mut prior = vec![0u16; NCTX * psize * 3];
        for t in 0..data.len() {
            let byte = data[t];
            let mut base = [0u64; NCTX];
            for (k, b) in base.iter_mut().enumerate() {
                *b = model_hash(data, t, k);
            }
            let mut node = 1u64;
            for bit in (0..8).rev() {
                let a = (byte >> bit) & 1;
                for (k, &bk) in base.iter().enumerate() {
                    let key = bk.wrapping_mul(MIXK).wrapping_add(node);
                    let idx = (key as usize) & pmask;
                    let chk = (key >> bits) as u16;
                    let off = (k * psize + idx) * 3;
                    if prior[off] != chk {
                        prior[off] = chk;
                        prior[off + 1] = 0;
                        prior[off + 2] = 0;
                    }
                    if a == 1 { prior[off + 2] += 1 } else { prior[off + 1] += 1 }
                    if prior[off + 1] + prior[off + 2] > 1024 {
                        prior[off + 1] = prior[off + 1].div_ceil(2);
                        prior[off + 2] = prior[off + 2].div_ceil(2);
                    }
                }
                node = (node << 1) | u64::from(a);
            }
        }
        prior
    }

    /// The table a device compress codes against by default, and its bits,
    /// built once per process: the compiled-in byte-ngram prior projected at
    /// `PRIOR_BITS` with slot counts capped at `PRIOR_CAP`, which a slot miss
    /// predicts from.
    fn default_prior() -> (&'static [u16], usize) {
        static P: OnceLock<Vec<u16>> = OnceLock::new();
        let table = P.get_or_init(|| project_prior(crate::seam::baked_model_blob(), PRIOR_BITS, PRIOR_CAP).0);
        (table, PRIOR_BITS)
    }

    /// The blob order device model `k` reads: its own order, or order 1 for
    /// the order-0 model. `None` for the word stem, whose context is a run of
    /// up to 32 bytes that no fixed-order row holds.
    fn projection_source(k: usize) -> Option<usize> {
        match (MODEL_KIND[k], MODEL_ORDER[k]) {
            (3, _) => None,
            (_, 0) => Some(1),
            (_, o) => Some(o),
        }
    }

    /// One order of a decoded byte-ngram blob: context bytes to next-byte
    /// counts, the shape `seam::decode_baked` returns.
    type BlobRows = std::collections::BTreeMap<Vec<u8>, std::collections::BTreeMap<u8, u32>>;

    /// Append the bit-tree nodes of one context row for model `k`: each
    /// next-byte count walked down the tree into (n0, n1) per node, keyed as
    /// the kernel keys that context. `ctx` is already folded or shaped for
    /// the model's kind.
    fn push_row_nodes(k: usize, ctx: &[u8], followers: impl Iterator<Item = (u8, u64)>, nodes: &mut Vec<(u64, u64, u64)>) {
        let mut h = FNV_OFFSET ^ (k as u64 + 1);
        for &b in ctx {
            h = (h ^ u64::from(b)).wrapping_mul(FNV_PRIME);
        }
        let mut n0 = [0u64; 256];
        let mut n1 = [0u64; 256];
        for (byte, c) in followers {
            let mut node = 1usize;
            for bit in (0..8).rev() {
                let a = (byte >> bit) & 1;
                if a == 1 { n1[node] += c } else { n0[node] += c }
                node = (node << 1) | usize::from(a);
            }
        }
        for (node, (&zeros, &ones)) in n0.iter().zip(&n1).enumerate().skip(1) {
            if zeros + ones > 0 {
                nodes.push((h.wrapping_mul(MIXK).wrapping_add(node as u64), zeros, ones));
            }
        }
    }

    /// Every bit-tree node device model `k` gets from `source`, the blob's
    /// rows at the model's source order, as (key, n0, n1).
    ///
    /// The walk is the one `corpus_prior` makes a byte at a time, so rows
    /// trained on a corpus give exactly the keys and counts that corpus gives
    /// the kernel's hashing. Rows that share a context once cut to the model's
    /// width and folded or shaped are summed first.
    fn projected_nodes(source: &BlobRows, k: usize) -> Vec<(u64, u64, u64)> {
        let width = MODEL_ORDER[k];
        let mut nodes = Vec::new();
        if MODEL_KIND[k] == 0 && projection_source(k) == Some(width) {
            for (ctx, followers) in source {
                push_row_nodes(k, ctx, followers.iter().map(|(&b, &c)| (b, u64::from(c))), &mut nodes);
            }
            return nodes;
        }
        let mut grouped: std::collections::BTreeMap<Vec<u8>, std::collections::BTreeMap<u8, u64>> =
            std::collections::BTreeMap::new();
        for (ctx, followers) in source {
            let key: Vec<u8> = ctx[ctx.len() - width..]
                .iter()
                .map(|&b| match MODEL_KIND[k] {
                    1 => fold_byte(b),
                    2 => shape_byte(b),
                    _ => b,
                })
                .collect();
            let sums = grouped.entry(key).or_default();
            for (&b, &c) in followers {
                *sums.entry(b).or_insert(0) += u64::from(c);
            }
        }
        for (ctx, sums) in &grouped {
            push_row_nodes(k, ctx, sums.iter().map(|(&b, &c)| (b, c)), &mut nodes);
        }
        nodes
    }

    /// Place `nodes` into one model's block of a device table at `bits`,
    /// heaviest first: a node takes its direct-mapped slot only when no
    /// heavier node took it, and its counts are halved until they total at
    /// most `cap`, as the kernel halves a slot past 1024. The block is
    /// cleared first. Returns the nodes placed and the count they carried
    /// before halving.
    ///
    /// Halving rounds up, so a slot with a count on each side never drops
    /// below 2: `cap` must be at least that.
    fn place_nodes(nodes: &mut [(u64, u64, u64)], block: &mut [u16], bits: usize, cap: u64) -> (usize, u64) {
        assert!(cap >= 2, "a slot cap below 2 cannot hold one count each way, and halving never reaches it");
        let psize = 1usize << bits;
        let pmask = psize - 1;
        assert_eq!(block.len(), psize * 3, "a block at {bits} bits holds {} u16s", psize * 3);
        nodes.sort_unstable_by(|a, b| (b.1 + b.2).cmp(&(a.1 + a.2)).then(a.0.cmp(&b.0)));
        block.fill(0);
        let mut taken = vec![false; psize];
        let mut placed = 0usize;
        let mut kept_mass = 0u64;
        for &(key, mut n0, mut n1) in nodes.iter() {
            let idx = (key as usize) & pmask;
            if taken[idx] {
                continue;
            }
            taken[idx] = true;
            placed += 1;
            kept_mass += n0 + n1;
            while n0 + n1 > cap {
                n0 = n0.div_ceil(2);
                n1 = n1.div_ceil(2);
            }
            let off = idx * 3;
            block[off] = (key >> bits) as u16;
            block[off + 1] = u16::try_from(n0).expect("a capped count fits a slot");
            block[off + 2] = u16::try_from(n1).expect("a capped count fits a slot");
        }
        (placed, kept_mass)
    }

    /// Project a serialized byte-ngram prior into the device table at `bits`,
    /// every model but the word stem from the blob's rows and the word stem
    /// from the counts `corpus_prior` gives it on `PRIOR_CORPUS`, since no row
    /// of the blob describes a stem context.
    pub(super) fn project_prior(blob: &[u8], bits: usize, cap: u64) -> (Vec<u16>, Vec<super::ProjectionStats>) {
        assert!((1..=28).contains(&bits), "a device prior needs 1 to 28 bits, not {bits}");
        assert!(cap >= 2, "a slot cap below 2 cannot hold one count each way, and halving never reaches it");
        let rows = crate::seam::decode_baked(blob);
        let psize = 1usize << bits;
        let mut prior = corpus_prior(PRIOR_CORPUS, bits);
        // One job per projected model, each owning its model's block of the
        // table, so the models build, sort and place in parallel and write
        // disjoint parts of it.
        struct ModelJob<'a> {
            k: usize,
            source: &'a BlobRows,
            block: &'a mut [u16],
            stats: Option<super::ProjectionStats>,
        }
        let mut jobs: Vec<ModelJob<'_>> = Vec::with_capacity(NCTX);
        for (k, block) in prior.chunks_mut(psize * 3).enumerate() {
            let Some(order) = projection_source(k) else {
                continue;
            };
            let Some(oi) = crate::seam::MODEL_ORDERS.iter().position(|&o| o == order) else {
                panic!("device model {k} reads order {order}, which the blob format does not carry");
            };
            let Some(source) = rows.get(oi) else {
                panic!("the blob decoded {} orders and has no order {order}", rows.len());
            };
            jobs.push(ModelJob { k, source, block, stats: None });
        }
        let plan = flynnel::JobPlan::new(0, jobs.len() as u32).with_leaf_shape(flynnel::LeafShape::Gather);
        flynnel::sched::par_iter::for_each_chunk_indexed_min_leaf(&plan, &mut jobs, 1, |_, slice| {
            for job in slice {
                let mut nodes = projected_nodes(job.source, job.k);
                let total_mass: u64 = nodes.iter().map(|n| n.1 + n.2).sum();
                let (placed, kept_mass) = place_nodes(&mut nodes, job.block, bits, cap);
                job.stats = Some(super::ProjectionStats {
                    model: job.k,
                    nodes: nodes.len(),
                    placed,
                    mass_kept: if total_mass == 0 { 1.0 } else { kept_mass as f64 / total_mass as f64 },
                });
            }
        });
        let stats = jobs.into_iter().map(|job| job.stats.expect("every model's job ran")).collect();
        (prior, stats)
    }

    /// The device context and the loaded compress kernel, initialized once.
    struct GpuCompress {
        ctx: Arc<CudaContext>,
        func: CudaFunction,
    }

    /// The process-wide compress kernel handle, or `None` when no device is
    /// usable, in which case coding runs on the CPU.
    fn gpu_compress() -> Option<&'static GpuCompress> {
        static GC: OnceLock<Option<GpuCompress>> = OnceLock::new();
        GC.get_or_init(|| load_or_cpu("compress", load_compress_kernel)).as_ref()
    }

    /// Device 0's context with the compress kernel loaded into it.
    fn load_compress_kernel() -> Result<GpuCompress, cudarc::driver::DriverError> {
        let ctx = CudaContext::new(0)?;
        let module = ctx.load_module(Ptx::from_src(COMPRESS_PTX))?;
        let func = module.load_function("trex_compress")?;
        Ok(GpuCompress { ctx, func })
    }

    /// Code `input` in KB-scale chunks, one per device thread, and return the
    /// summed code length in bits.
    pub(super) fn compress(input: &[u8]) -> Option<f64> {
        match held_default_prior() {
            Some(held) => compress_held(input, held),
            None => compress_with(input, &[0u16; 3], 0, false),
        }
    }

    /// [`compress`] reading `prior`, a table at `prior_bits`; zero bits seeds
    /// every slot uniform and never reads the table. With `predict`, a slot miss
    /// predicts from the prior as well as seeding the slot. The table is
    /// uploaded for this call alone.
    pub(super) fn compress_with(input: &[u8], prior: &[u16], prior_bits: usize, predict: bool) -> Option<f64> {
        let held = hold_prior(prior, prior_bits, predict)?;
        compress_held(input, &held)
    }

    /// A device prior table on the device, with the bits it was built at and
    /// whether a slot miss predicts from it.
    pub(super) struct HeldPrior {
        d_prior: cudarc::driver::CudaSlice<u16>,
        bits: usize,
        predict: bool,
    }

    /// Upload `prior`, a table at `prior_bits`, to be held; zero bits takes a
    /// non-empty table the kernel never reads. With `predict`, a slot miss
    /// predicts from the table as well as seeding the slot. A failed upload is
    /// reported before it becomes `None`.
    pub(super) fn hold_prior(prior: &[u16], prior_bits: usize, predict: bool) -> Option<HeldPrior> {
        let want = if prior_bits == 0 { prior.len() } else { NCTX * (1usize << prior_bits) * 3 };
        assert!(
            !prior.is_empty() && prior.len() == want,
            "a prior at {prior_bits} bits holds {want} u16s, not {}",
            prior.len()
        );
        let g = gpu_compress()?;
        match g.ctx.default_stream().clone_htod(prior) {
            Ok(d_prior) => Some(HeldPrior { d_prior, bits: prior_bits, predict }),
            Err(e) => {
                eprintln!("trex gpu compress: holding the {}-u16 prior failed: {e:?}", prior.len());
                None
            }
        }
    }

    /// The default prior held on the device for the life of the process,
    /// uploaded on the first compress. `None` when no usable device is present
    /// or the device would not hold the table, which [`hold_prior`] reports;
    /// [`compress`] then codes with no prior.
    fn held_default_prior() -> Option<&'static HeldPrior> {
        static HELD: OnceLock<Option<HeldPrior>> = OnceLock::new();
        HELD.get_or_init(hold_default_prior).as_ref()
    }

    /// The prior [`compress`] codes against, held and read as it reads it.
    pub(super) fn hold_default_prior() -> Option<HeldPrior> {
        let (table, bits) = default_prior();
        hold_prior(table, bits, bits > 0)
    }

    /// Code `input` in KB-scale chunks against a held prior and return the
    /// summed code length in bits. A device failure is reported before it
    /// becomes `None`.
    pub(super) fn compress_held(input: &[u8], held: &HeldPrior) -> Option<f64> {
        let g = gpu_compress()?;
        let n = input.len();
        if n == 0 {
            return Some(0.0);
        }
        // Chunk size trades ratio (bigger = more context per chunk, better
        // ratio) against parallelism (more chunks = more device threads). The
        // default scales with the input - n/4096 aims at thousands of chunks on
        // large inputs (device saturation) while the [CHUNK, 64K] clamp keeps
        // small inputs parallel and large chunks context-rich. TREX_GPU_CHUNK
        // overrides for sweeps.
        let chunk: usize = env_or("TREX_GPU_CHUNK", (n / 4096).clamp(CHUNK, 65536));
        assert!(chunk > 0, "TREX_GPU_CHUNK must be positive");
        let nc = n.div_ceil(chunk);
        let starts: Vec<i32> = (0..nc).map(|i| (i * chunk) as i32).collect();
        let ends: Vec<i32> = (0..nc).map(|i| ((i + 1) * chunk).min(n) as i32).collect();
        // Size the per-thread tables to the chunk (~16x its bytes, like the CPU
        // coder's input-proportional sizing), shrinking to fit a 2 GB device
        // budget - undersized tables thrash with evictions at large chunks.
        let mut ctx_bits = (chunk.max(1).ilog2() as usize + 4).clamp(14, 20);
        while ctx_bits > 14 && nc * NCTX * (1usize << ctx_bits) * 6 > (2usize << 30) {
            ctx_bits -= 1;
        }
        let nc_i = nc as i32;
        let ctx_bits_i = ctx_bits as i32;
        let prior_bits_i = held.bits as i32;
        let prior_predict_i = i32::from(held.predict);
        // Overlapping context: warm each chunk on this many real preceding bytes
        // (tunable; default half the chunk). The chunk boundaries stay fixed, so
        // only the warm-up cost grows - the ratio gain is real local context.
        let overlap: i32 = env_or("TREX_GPU_OVERLAP", (chunk / 2) as i32);
        let block = 128u32;
        let cfg = LaunchConfig {
            grid_dim: (nc.div_ceil(block as usize) as u32, 1, 1),
            block_dim: (block, 1, 1),
            shared_mem_bytes: 0,
        };

        let stream = g.ctx.default_stream();
        let ran = (|| {
            let d_input = stream.clone_htod(input)?;
            let d_start = stream.clone_htod(&starts)?;
            let d_end = stream.clone_htod(&ends)?;
            // The kernel reads each thread's table block as zeroed; alloc_zeros
            // gives exactly that. Three u16 per slot matches the kernel's Slot;
            // the match hash is one u32 (a position + 1, 0 meaning empty) per
            // bucket.
            let d_tables = stream.alloc_zeros::<u16>(nc * NCTX * (1 << ctx_bits) * 3)?;
            let d_mtables = stream.alloc_zeros::<u32>(nc * MATCH_SIZE)?;
            // Per-thread embedding store (the kernel seeds it) and node vectors
            // (zero-init). ~2 MB/thread of embeddings.
            let d_emb = stream.alloc_zeros::<i32>(nc * NEMB * EMB_SIZE * EMB_DIM)?;
            let d_wnode = stream.alloc_zeros::<i32>(nc * NEMB * 256 * EMB_DIM)?;
            // Per-selector mixer weights (the kernel inits them).
            // NIN = contexts + match + embeddings + neural trace.
            let d_weights = stream.alloc_zeros::<i32>(nc * 256 * (NCTX + 1 + NEMB + 1))?;
            let d_out = stream.alloc_zeros::<f64>(nc)?;
            let mut builder = stream.launch_builder(&g.func);
            builder.arg(&d_input);
            builder.arg(&d_start);
            builder.arg(&d_end);
            builder.arg(&nc_i);
            builder.arg(&held.d_prior);
            builder.arg(&prior_bits_i);
            builder.arg(&prior_predict_i);
            builder.arg(&overlap);
            builder.arg(&ctx_bits_i);
            builder.arg(&d_tables);
            builder.arg(&d_mtables);
            builder.arg(&d_emb);
            builder.arg(&d_wnode);
            builder.arg(&d_weights);
            builder.arg(&d_out);
            // SAFETY: the argument list matches `trex_compress`'s parameters in
            // order and type; the tables and match buffers are sized to the
            // kernel's per-thread blocks, and the held prior to `held.bits`,
            // which the kernel never indexes past.
            unsafe { builder.launch(cfg)? };
            let out: Vec<f64> = stream.clone_dtoh(&d_out)?;
            stream.synchronize()?;
            Ok::<_, cudarc::driver::DriverError>(out)
        })();
        match ran {
            Ok(out) => Some(out.iter().sum()),
            Err(e) => {
                eprintln!("trex gpu compress: coding {n} bytes in {nc} chunks failed: {e:?}");
                None
            }
        }
    }

    /// A sweep override read from the environment, or `default` when the
    /// variable is unset. A variable that is set but is not valid Unicode, or
    /// does not parse, panics naming it: a sweep that mistyped its override
    /// would otherwise run on the default and report numbers for a setting it
    /// never used.
    fn env_or<T: std::str::FromStr>(name: &str, default: T) -> T
    where
        T::Err: std::fmt::Display,
    {
        match std::env::var_os(name) {
            None => default,
            Some(raw) => match raw.to_str() {
                None => panic!("{name} is set but is not valid Unicode"),
                Some(s) => match s.parse::<T>() {
                    Ok(v) => v,
                    Err(e) => panic!("{name}={s:?} does not parse: {e}"),
                },
            },
        }
    }

    /// [`compress`] with a clock between its phases, for the phase report.
    ///
    /// The same sequence as `compress`, so the code length must match it; the
    /// example that prints this asserts that before reading any timing. Each
    /// group of copies or allocations ends in a synchronize, since both are
    /// queued on the stream and would otherwise land in whichever phase next
    /// waits. Device failures are reported before they become `None`.
    pub(super) fn compress_phases(input: &[u8]) -> Option<(f64, super::CompressPhases)> {
        use std::time::Instant;

        let g = gpu_compress()?;
        let mut ph = super::CompressPhases::default();
        let n = input.len();
        if n == 0 {
            return Some((0.0, ph));
        }

        let t = Instant::now();
        let chunk: usize = env_or("TREX_GPU_CHUNK", (n / 4096).clamp(CHUNK, 65536));
        assert!(chunk > 0, "TREX_GPU_CHUNK must be positive");
        let nc = n.div_ceil(chunk);
        let starts: Vec<i32> = (0..nc).map(|i| (i * chunk) as i32).collect();
        let ends: Vec<i32> = (0..nc).map(|i| ((i + 1) * chunk).min(n) as i32).collect();
        let (prior, prior_bits) = default_prior();
        ph.host_prep_us = t.elapsed().as_secs_f64() * 1e6;
        ph.chunks = nc;
        ph.prior_bytes = std::mem::size_of_val(prior);

        let stream = g.ctx.default_stream();
        let report = |what: &str, e: &cudarc::driver::DriverError| {
            eprintln!("trex gpu compress probe: {what} for {n} bytes failed: {e:?}");
        };

        let t = Instant::now();
        let staged = (|| {
            let d_input = stream.clone_htod(input)?;
            let d_start = stream.clone_htod(&starts)?;
            let d_end = stream.clone_htod(&ends)?;
            stream.synchronize()?;
            Ok::<_, cudarc::driver::DriverError>((d_input, d_start, d_end))
        })();
        let (d_input, d_start, d_end) = match staged {
            Ok(v) => v,
            Err(e) => {
                report("uploading the input", &e);
                return None;
            }
        };
        ph.upload_input_us = t.elapsed().as_secs_f64() * 1e6;

        let mut ctx_bits = (chunk.ilog2() as usize + 4).clamp(14, 20);
        while ctx_bits > 14 && nc * NCTX * (1usize << ctx_bits) * 6 > (2usize << 30) {
            ctx_bits -= 1;
        }
        let t = Instant::now();
        let allocated = (|| {
            let d_tables = stream.alloc_zeros::<u16>(nc * NCTX * (1 << ctx_bits) * 3)?;
            let d_mtables = stream.alloc_zeros::<u32>(nc * MATCH_SIZE)?;
            let d_emb = stream.alloc_zeros::<i32>(nc * NEMB * EMB_SIZE * EMB_DIM)?;
            let d_wnode = stream.alloc_zeros::<i32>(nc * NEMB * 256 * EMB_DIM)?;
            let d_weights = stream.alloc_zeros::<i32>(nc * 256 * (NCTX + 1 + NEMB + 1))?;
            let d_out = stream.alloc_zeros::<f64>(nc)?;
            stream.synchronize()?;
            Ok::<_, cudarc::driver::DriverError>((d_tables, d_mtables, d_emb, d_wnode, d_weights, d_out))
        })();
        let (d_tables, d_mtables, d_emb, d_wnode, d_weights, d_out) = match allocated {
            Ok(v) => v,
            Err(e) => {
                report("allocating the per-thread tables", &e);
                return None;
            }
        };
        ph.alloc_us = t.elapsed().as_secs_f64() * 1e6;

        let t = Instant::now();
        let uploaded = (|| {
            let d = stream.clone_htod(prior)?;
            stream.synchronize()?;
            Ok::<_, cudarc::driver::DriverError>(d)
        })();
        let d_prior = match uploaded {
            Ok(d) => d,
            Err(e) => {
                report("uploading the prior", &e);
                return None;
            }
        };
        ph.upload_prior_us = t.elapsed().as_secs_f64() * 1e6;

        let nc_i = nc as i32;
        let prior_bits_i = prior_bits as i32;
        let prior_predict_i = i32::from(prior_bits > 0);
        let overlap: i32 = env_or("TREX_GPU_OVERLAP", (chunk / 2) as i32);
        let block = 128u32;
        let cfg = LaunchConfig {
            grid_dim: (nc.div_ceil(block as usize) as u32, 1, 1),
            block_dim: (block, 1, 1),
            shared_mem_bytes: 0,
        };
        let ctx_bits_i = ctx_bits as i32;

        let t = Instant::now();
        let ran = (|| {
            let mut builder = stream.launch_builder(&g.func);
            builder.arg(&d_input);
            builder.arg(&d_start);
            builder.arg(&d_end);
            builder.arg(&nc_i);
            builder.arg(&d_prior);
            builder.arg(&prior_bits_i);
            builder.arg(&prior_predict_i);
            builder.arg(&overlap);
            builder.arg(&ctx_bits_i);
            builder.arg(&d_tables);
            builder.arg(&d_mtables);
            builder.arg(&d_emb);
            builder.arg(&d_wnode);
            builder.arg(&d_weights);
            builder.arg(&d_out);
            // SAFETY: as in `compress` - the argument list matches
            // `trex_compress`'s parameters in order and type, and every table
            // is sized to the kernel's per-thread blocks.
            unsafe { builder.launch(cfg)? };
            let out: Vec<f64> = stream.clone_dtoh(&d_out)?;
            stream.synchronize()?;
            Ok::<_, cudarc::driver::DriverError>(out)
        })();
        let out = match ran {
            Ok(o) => o,
            Err(e) => {
                report("running the kernel", &e);
                return None;
            }
        };
        ph.kernel_and_download_us = t.elapsed().as_secs_f64() * 1e6;
        Some((out.iter().sum(), ph))
    }

    #[cfg(test)]
    mod tests {
        use std::collections::BTreeMap;

        use super::*;

        /// Mixed case, digits, punctuation and repeats, so every model kind
        /// has contexts to project and some rows share a folded or shaped
        /// context.
        const SAMPLE: &[u8] = b"The cat sat. the Cat ran 42 times; THE CAT sat again, and 7 cats sat by 42 mats.";

        #[test]
        fn projected_nodes_are_the_counts_the_kernel_hashing_gives_the_corpus() {
            let corpus = SAMPLE.repeat(3);
            let rows = crate::seam::byte_ngram_train(&corpus);
            let mut projected_models = 0;
            for k in 0..NCTX {
                let Some(order) = projection_source(k) else {
                    continue;
                };
                let oi = crate::seam::MODEL_ORDERS
                    .iter()
                    .position(|&o| o == order)
                    .expect("every source order is one the trainer counts");
                let mut want: BTreeMap<u64, (u64, u64)> = BTreeMap::new();
                for t in order..corpus.len() {
                    let base = model_hash(&corpus, t, k);
                    let mut node = 1u64;
                    for bit in (0..8).rev() {
                        let a = (corpus[t] >> bit) & 1;
                        let e = want.entry(base.wrapping_mul(MIXK).wrapping_add(node)).or_insert((0, 0));
                        if a == 1 { e.1 += 1 } else { e.0 += 1 }
                        node = (node << 1) | u64::from(a);
                    }
                }
                let mut got: BTreeMap<u64, (u64, u64)> = BTreeMap::new();
                for (key, n0, n1) in projected_nodes(&rows[oi], k) {
                    assert!(got.insert(key, (n0, n1)).is_none(), "model {k} projected key {key:#x} twice");
                }
                assert!(!want.is_empty(), "model {k} counted nothing on the sample");
                assert_eq!(got, want, "model {k} projected counts the corpus does not give it");
                projected_models += 1;
            }
            assert_eq!(projected_models, NCTX - 1, "every model but the word stem projects");
        }

        #[test]
        fn a_heavier_node_takes_the_slot_and_its_counts_halve_to_the_cap() {
            let bits = 4;
            let mut block = vec![7u16; 16 * 3];
            let mut nodes = vec![(0x103u64, 6u64, 4u64), (0x203, 1500, 500), (0x305, 0, 7)];
            let (placed, kept) = place_nodes(&mut nodes, &mut block, bits, 1024);
            assert_eq!((placed, kept), (2, 2007));
            assert_eq!(&block[3 * 3..4 * 3], &[0x20, 750, 250], "the 2000-count node wins slot 3, halved once");
            assert_eq!(&block[5 * 3..6 * 3], &[0x30, 0, 7], "a node under the cap keeps its counts");
            let untouched: usize = (0..16).filter(|&i| i != 3 && i != 5).map(|i| usize::from(block[i * 3 + 1])).sum();
            assert_eq!(untouched, 0, "every other slot is cleared");
        }

        #[test]
        fn the_smallest_cap_halves_a_two_sided_slot_to_one_count_each_way() {
            let mut block = vec![0u16; 16 * 3];
            let mut nodes = vec![(0x203u64, 1500u64, 500u64), (0x305, 3, 0)];
            let (placed, _) = place_nodes(&mut nodes, &mut block, 4, 2);
            assert_eq!(placed, 2);
            assert_eq!(&block[3 * 3..4 * 3], &[0x20, 1, 1], "halving rounds up, so both sides keep one count");
            assert_eq!(&block[5 * 3..6 * 3], &[0x30, 2, 0], "a one-sided slot halves to the cap");
        }
    }
}
