//! Counting a byte on the device against counting it on the cores.
//!
//! A numbered tail counts every newline ahead of its last lines, which for a
//! large file is nearly all of it, and the cores do it with the vector count.
//! A device counts faster than the cores read, but the bytes are in host
//! memory, where a file read leaves them, so a count there first copies them
//! to the device. This times the whole of that: the copy, a count kernel over
//! the copied bytes, and the one number read back; beside it the kernel alone
//! over bytes already resident, which is the most a device could offer, and
//! the cores' one-pass and split counts over the same bytes.
//!
//! Sizes run from 1 MiB to 256 MiB of the corpus the crossover benches build,
//! and each file named is counted at its opening sizes and whole. Every form's
//! count is checked against the scalar count.
//!
//! Run: `cargo bench --bench count_byte_device [-- FILE...]`.

use std::hint::black_box;
use std::time::Instant;

use cudarc::driver::{CudaContext, CudaFunction, CudaStream, LaunchConfig, PushKernelArg};

/// A grid-stride count over 16-byte words, each 32-bit lane counted with the
/// zero-byte test on the lane XORed against the byte repeated four times; the
/// tail past the last whole word is counted a byte at a time. Each warp sums
/// its lanes and adds its total to the one counter.
const KERNEL: &str = r#"
__device__ unsigned int zero_bytes(unsigned int x) {
    return __popc(~(((x & 0x7F7F7F7Fu) + 0x7F7F7F7Fu) | x | 0x7F7F7F7Fu));
}

extern "C" __global__ void count_byte(const unsigned char* data, unsigned long long n,
                                      unsigned int byte, unsigned long long* out) {
    unsigned long long tid = blockIdx.x * (unsigned long long)blockDim.x + threadIdx.x;
    unsigned long long stride = (unsigned long long)gridDim.x * blockDim.x;
    unsigned long long words = n / 16;
    unsigned int pattern = byte * 0x01010101u;
    const uint4* v = (const uint4*)data;
    unsigned long long total = 0;
    for (unsigned long long i = tid; i < words; i += stride) {
        uint4 w = v[i];
        total += zero_bytes(w.x ^ pattern) + zero_bytes(w.y ^ pattern)
               + zero_bytes(w.z ^ pattern) + zero_bytes(w.w ^ pattern);
    }
    for (unsigned long long i = words * 16 + tid; i < n; i += stride) {
        total += data[i] == byte;
    }
    for (int off = 16; off > 0; off >>= 1) {
        total += __shfl_down_sync(0xffffffffu, total, off);
    }
    if ((threadIdx.x & 31) == 0) {
        atomicAdd(out, total);
    }
}
"#;

/// The corpus `benches/vs_regex_full.rs` builds, cut to `bytes`.
fn corpus(bytes: usize) -> Vec<u8> {
    let mut s = String::new();
    let mut i = 0usize;
    while s.len() < bytes {
        match i % 4 {
            0 => s.push_str(&format!("let value_{i} = {} ;\n", i * 37)),
            1 => s.push_str(&format!("call_{i}(alpha, beta, {}) ;\n", i)),
            2 => s.push_str(&format!("key_{i}: item_{i}, item_{}, item_{} ;\n", i + 1, i + 2)),
            _ => s.push_str(&format!("if (cond_{i}) {{ do_{i}(x) ; }}\n")),
        }
        i += 1;
    }
    let mut v = s.into_bytes();
    v.truncate(bytes);
    v
}

const ROUNDS: usize = 7;
const BUDGET_MS: f64 = 50.0;
const SIZES: [usize; 5] =
    [1024 * 1024, 4 * 1024 * 1024, 16 * 1024 * 1024, 64 * 1024 * 1024, 256 * 1024 * 1024];
const BLOCK: u32 = 256;
const GRID: u32 = 1024;

fn time_budget(f: &mut dyn FnMut() -> u64) -> f64 {
    black_box(f());
    let t0 = Instant::now();
    let mut iters = 0u32;
    loop {
        black_box(f());
        iters += 1;
        if iters >= 3 && t0.elapsed().as_secs_f64() * 1e3 >= BUDGET_MS {
            break;
        }
    }
    t0.elapsed().as_secs_f64() * 1e3 / f64::from(iters)
}

fn median(mut v: Vec<f64>) -> f64 {
    v.sort_by(f64::total_cmp);
    v[v.len() / 2]
}

/// The device, its stream and the count kernel.
struct Device {
    stream: std::sync::Arc<CudaStream>,
    func: CudaFunction,
}

impl Device {
    fn open() -> Device {
        let ctx = match CudaContext::new(0) {
            Ok(ctx) => ctx,
            Err(e) => {
                eprintln!("count_byte_device: no CUDA device 0: {e:?}");
                std::process::exit(3);
            }
        };
        let ptx = match cudarc::nvrtc::compile_ptx(KERNEL) {
            Ok(ptx) => ptx,
            Err(e) => {
                eprintln!("count_byte_device: the count kernel does not compile: {e:?}");
                std::process::exit(3);
            }
        };
        let func = match ctx.load_module(ptx).and_then(|m| m.load_function("count_byte")) {
            Ok(func) => func,
            Err(e) => {
                eprintln!("count_byte_device: the count kernel does not load: {e:?}");
                std::process::exit(3);
            }
        };
        Device { stream: ctx.default_stream(), func }
    }

    /// The count kernel over `data`, resident, and the count read back.
    fn count_resident(&self, data: &cudarc::driver::CudaSlice<u8>, n: u64, byte: u8) -> u64 {
        let run = || {
            let out = self.stream.alloc_zeros::<u64>(1)?;
            let byte = u32::from(byte);
            let cfg = LaunchConfig { grid_dim: (GRID, 1, 1), block_dim: (BLOCK, 1, 1), shared_mem_bytes: 0 };
            let mut builder = self.stream.launch_builder(&self.func);
            builder.arg(data);
            builder.arg(&n);
            builder.arg(&byte);
            builder.arg(&out);
            // SAFETY: the arguments match `count_byte`'s parameters in order and
            // type; `data` holds `n` bytes and `out` one counter, and the kernel
            // reads no byte past `n` and writes only the counter.
            unsafe { builder.launch(cfg)? };
            let counted = self.stream.clone_dtoh(&out)?;
            self.stream.synchronize()?;
            Ok::<_, cudarc::driver::DriverError>(counted[0])
        };
        match run() {
            Ok(counted) => counted,
            Err(e) => {
                eprintln!("count_byte_device: the count kernel failed over {n} bytes: {e:?}");
                std::process::exit(4);
            }
        }
    }

    /// `input` copied to the device, counted there, and the count read back.
    fn count_copied(&self, input: &[u8], byte: u8) -> u64 {
        let data = match self.stream.clone_htod(input) {
            Ok(data) => data,
            Err(e) => {
                eprintln!("count_byte_device: copying {} bytes to the device failed: {e:?}", input.len());
                std::process::exit(4);
            }
        };
        self.count_resident(&data, input.len() as u64, byte)
    }
}

/// One way of counting, by its name.
type Form<'a> = (&'static str, Box<dyn FnMut() -> u64 + 'a>);

/// The four forms over one input, in an order that rotates each round so
/// none reads its position as its own cost, and the first read once more after
/// every round as the control.
fn row(device: &Device, source: &str, input: &[u8]) {
    let byte = b'\n';
    let expected = trex::byte_simd::count_byte_scalar(input, byte) as u64;
    let resident = match device.stream.clone_htod(input) {
        Ok(data) => data,
        Err(e) => {
            eprintln!("count_byte_device: copying {} bytes to the device failed: {e:?}", input.len());
            std::process::exit(4);
        }
    };
    let n = input.len() as u64;
    let mut forms: Vec<Form<'_>> = vec![
        ("one pass", Box::new(|| trex::byte_simd::count_byte(input, byte) as u64)),
        ("across", Box::new(|| trex::byte_simd::count_byte_across(input, byte) as u64)),
        ("copy+count", Box::new(|| device.count_copied(input, byte))),
        ("resident", Box::new(|| device.count_resident(&resident, n, byte))),
    ];
    for (name, f) in &mut forms {
        let got = f();
        assert_eq!(got, expected, "{name} counted {got} of the {expected} newlines in {source}");
    }
    let mut times: Vec<Vec<f64>> = vec![Vec::new(); forms.len()];
    for r in 0..ROUNDS {
        for k in 0..forms.len() {
            let at = (k + r) % forms.len();
            times[at].push(time_budget(&mut *forms[at].1));
        }
    }
    let control = time_budget(&mut *forms[0].1);
    let ms: Vec<f64> = times.into_iter().map(median).collect();
    let gbps = |t: f64| input.len() as f64 / (t * 1e6);
    println!(
        "{source:>24} {:>11} {:>10.4} {:>10.4} {:>10.4} {:>10.4} {:>7.1} {:>7.1} {:>7.1} {:>7.1} {:>8.3}x",
        input.len(),
        ms[0],
        ms[1],
        ms[2],
        ms[3],
        gbps(ms[0]),
        gbps(ms[1]),
        gbps(ms[2]),
        gbps(ms[3]),
        control / ms[0]
    );
}

fn main() {
    let device = Device::open();
    let cores = std::thread::available_parallelism().map_or(1, std::num::NonZero::get);
    println!("trex {}, {cores} cores, CUDA device 0", trex::version());
    println!(
        "{:>24} {:>11} {:>10} {:>10} {:>10} {:>10} {:>7} {:>7} {:>7} {:>7} {:>9}",
        "source", "bytes", "one ms", "across ms", "copy ms", "dev ms", "one", "across", "copy", "dev", "control"
    );
    println!("{:>24} {:>11} {:>43} {:>31}", "", "", "(copy = to the device, counted, read back)", "GB/s of each form");
    for bytes in SIZES {
        row(&device, "corpus", &corpus(bytes));
    }
    for path in std::env::args().skip(1).filter(|a| !a.starts_with('-')) {
        let whole = match std::fs::read(&path) {
            Ok(bytes) => bytes,
            Err(e) => {
                eprintln!("count_byte_device: cannot read {path}: {e}");
                std::process::exit(2);
            }
        };
        let name = std::path::Path::new(&path)
            .file_name()
            .map_or_else(|| path.clone(), |n| n.to_string_lossy().into_owned());
        for bytes in SIZES.iter().copied().filter(|&b| b <= whole.len()) {
            row(&device, &name, &whole[..bytes]);
        }
        row(&device, &name, &whole);
    }
}
