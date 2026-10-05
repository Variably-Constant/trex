//! Build script: bake the CUDA kernels into the binary.
//!
//! The `gpu` feature is on by default. When it is on and an `nvcc` of CUDA
//! 12.0 or later is available, `nvcc` compiles `kernels/scan.cu` to PTX
//! targeting the `compute_75` virtual architecture (the oldest the CUDA 13
//! toolkit emits; the driver JIT-compiles it up to the running device), and
//! the PTX path is exported so the source embeds it and the runtime loads it
//! through the driver - the deployed binary then needs only an NVIDIA driver
//! of the series the toolkit's PTX asks for. `kernels/compress.cu`, the
//! device coder, is baked the same way only when the `compress` feature is
//! on as well. The `nvcc` used is the one on PATH, else `$CUDA_PATH/bin`,
//! else the newest installed toolkit, so a builder picks a toolkit by putting
//! it first.
//!
//! When no such `nvcc` is found, or it fails, this writes a placeholder
//! instead of failing, so a default build still succeeds and runs CPU-only.
//! With `TREX_REQUIRE_CUDA=1`, which trex's own release builds set, a Windows
//! or Linux build fails instead, so no release for those ships without the
//! kernel. With the feature off (`--no-default-features`) this is a no-op
//! and the crate builds as pure Rust with no CUDA involvement.

use std::path::{Path, PathBuf};
use std::process::Command;

/// The oldest CUDA toolkit whose `nvcc` bakes the kernels: its PTX loads on
/// drivers from the 525 series up.
const NVCC_FLOOR: (u32, u32) = (12, 0);

fn main() {
    println!("cargo:rerun-if-changed=kernels/scan.cu");
    println!("cargo:rerun-if-changed=kernels/compress.cu");
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-env-changed=TREX_REQUIRE_CUDA");
    println!("cargo:rerun-if-env-changed=CUDA_PATH");

    // Pure-CPU build: the gpu feature is off (--no-default-features), so
    // there is no kernel to compile.
    if std::env::var_os("CARGO_FEATURE_GPU").is_none() {
        return;
    }

    let out_dir = PathBuf::from(std::env::var("OUT_DIR").expect("OUT_DIR set by cargo"));
    // A release build of trex for a platform CUDA ships on must carry the
    // kernel; macOS and FreeBSD have no CUDA toolkit and build CPU-only.
    let target_os = std::env::var("CARGO_CFG_TARGET_OS").expect("CARGO_CFG_TARGET_OS set by cargo");
    let required = std::env::var_os("TREX_REQUIRE_CUDA").is_some_and(|v| v == "1")
        && matches!(target_os.as_str(), "windows" | "linux");

    // gpu is on by default. Bake each kernel; where that fails - no nvcc of
    // the floor or later, or nvcc unable to run (no cl.exe on Windows without
    // vcvars) - write a placeholder so the crate still builds and runs
    // CPU-only, unless the build requires the kernel. The runtime's module
    // load rejects the placeholder, so the device path is simply unavailable.
    let mut warned = false;
    let mut kernels = vec![("kernels/scan.cu", "scan.ptx", "TREX_SCAN_PTX")];
    if std::env::var_os("CARGO_FEATURE_COMPRESS").is_some() {
        kernels.push(("kernels/compress.cu", "compress.ptx", "TREX_COMPRESS_PTX"));
    }
    for (src, out_name, env_var) in kernels {
        let ptx_path = out_dir.join(out_name);
        if let Err(why) = bake_ptx(src, &ptx_path) {
            assert!(
                !required,
                "trex: TREX_REQUIRE_CUDA=1 and {src} was not baked: {why}. A release build for Windows or \
                 Linux carries the CUDA kernel: install CUDA {}.{} or later and put its nvcc first on PATH, \
                 and on Windows build where Visual Studio's vcvars is found so nvcc finds cl.exe.",
                NVCC_FLOOR.0,
                NVCC_FLOOR.1
            );
            std::fs::write(&ptx_path, "// trex: no CUDA kernel baked at build time\n")
                .expect("write placeholder PTX to OUT_DIR");
            if !warned {
                println!(
                    "cargo:warning=trex: could not bake the CUDA kernels ({why}; on Windows build from a \
                     shell with vcvars so nvcc finds cl.exe). The binary is CPU-only. Build with \
                     --no-default-features for pure CPU."
                );
                warned = true;
            }
        }
        println!("cargo:rustc-env={env_var}={}", ptx_path.display());
    }
}

/// Compile the CUDA source `src` to PTX at `ptx_path` via `nvcc`, or say why
/// it could not be: no `nvcc` found, one older than [`NVCC_FLOOR`], or `nvcc`
/// failing, on Windows inside the vcvars environment as well, so the caller
/// falls back to a CPU-only build or fails one that requires the kernel.
fn bake_ptx(src: &str, ptx_path: &Path) -> Result<(), String> {
    let nvcc = find_nvcc().ok_or_else(|| "no nvcc found".to_string())?;
    let version = nvcc_version(&nvcc)?;
    if version < NVCC_FLOOR {
        return Err(format!(
            "{} is CUDA {}.{}, older than {}.{}",
            nvcc.display(),
            version.0,
            version.1,
            NVCC_FLOOR.0,
            NVCC_FLOOR.1
        ));
    }
    let ptx = ptx_path.to_string_lossy().into_owned();
    let direct = Command::new(&nvcc).args(["-ptx", "-arch=compute_75", "-o", ptx.as_str(), src]).status();
    if matches!(direct, Ok(s) if s.success()) && ptx_path.exists() {
        return Ok(());
    }
    // On Windows nvcc needs cl.exe + its INCLUDE/LIB, which a plain shell lacks; retry inside the
    // standard vcvars environment (auto-located under the VS install dirs) so the bake works without
    // the caller first opening a developer shell.
    #[cfg(windows)]
    if let Some((vcvars, dir)) = find_vcvars().zip(ptx_path.parent()) {
        // cmd /C mangles a `"quoted path" && "quoted path"` line; a .bat file runs cleanly. `call`
        // the vcvars script (sets cl.exe + INCLUDE/LIB) then invoke nvcc in that environment.
        let bat = dir.join("trex_bake.bat");
        let script = format!("@echo off\r\ncall \"{}\" >nul\r\n\"{}\" -ptx -arch=compute_75 -o \"{ptx}\" \"{src}\"\r\n", vcvars.display(), nvcc.display());
        std::fs::write(&bat, script).map_err(|e| format!("cannot write {}: {e}", bat.display()))?;
        return match Command::new("cmd").arg("/C").arg(&bat).status() {
            Ok(s) if s.success() && ptx_path.exists() => Ok(()),
            Ok(s) => Err(format!("{} exited with {s} inside vcvars", nvcc.display())),
            Err(e) => Err(format!("cannot run cmd for vcvars: {e}")),
        };
    }
    match direct {
        Ok(s) => Err(format!("{} exited with {s}", nvcc.display())),
        Err(e) => Err(format!("cannot run {}: {e}", nvcc.display())),
    }
}

/// The CUDA release `nvcc --version` names, as major and minor: the
/// `release 13.3` of `Cuda compilation tools, release 13.3, V13.3.73`.
fn nvcc_version(nvcc: &Path) -> Result<(u32, u32), String> {
    let out = Command::new(nvcc)
        .arg("--version")
        .output()
        .map_err(|e| format!("cannot run {} --version: {e}", nvcc.display()))?;
    let text = String::from_utf8_lossy(&out.stdout);
    let release = text
        .split("release ")
        .nth(1)
        .and_then(|after| after.split([',', ' ', '\r', '\n']).next())
        .ok_or_else(|| format!("{} --version names no CUDA release", nvcc.display()))?;
    let unread = |why: String| format!("{} --version names release {release:?}: {why}", nvcc.display());
    let (major, minor) = release.split_once('.').ok_or_else(|| unread("not MAJOR.MINOR".to_string()))?;
    let major = major.parse::<u32>().map_err(|e| unread(e.to_string()))?;
    let minor = minor.parse::<u32>().map_err(|e| unread(e.to_string()))?;
    Ok((major, minor))
}

/// Locate `nvcc`: PATH, then `$CUDA_PATH/bin`, then the standard CUDA-toolkit install dirs (highest
/// version), so a normal build finds it without a developer shell or a manual PATH edit.
fn find_nvcc() -> Option<PathBuf> {
    if Command::new("nvcc").arg("--version").output().map(|o| o.status.success()).unwrap_or(false) {
        return Some(PathBuf::from("nvcc"));
    }
    let exe = if cfg!(windows) { "nvcc.exe" } else { "nvcc" };
    if let Some(cuda) = std::env::var_os("CUDA_PATH") {
        let p = PathBuf::from(cuda).join("bin").join(exe);
        if p.exists() {
            return Some(p);
        }
    }
    // Unversioned Linux roots.
    for direct in ["/usr/local/cuda", "/opt/cuda"] {
        let p = PathBuf::from(direct).join("bin").join(exe);
        if p.exists() {
            return Some(p);
        }
    }
    // Versioned install roots (Windows CUDA\v13.1\... ; Linux cuda-13.1) - pick the highest version.
    let roots: &[&str] = if cfg!(windows) { &[r"C:\Program Files\NVIDIA GPU Computing Toolkit\CUDA"] } else { &["/usr/local"] };
    let ver = |name: &str| -> (u32, u32) {
        let s = name.trim_start_matches('v').trim_start_matches("cuda-");
        let mut it = s.split('.').map(|x| x.parse::<u32>().unwrap_or(0));
        (it.next().unwrap_or(0), it.next().unwrap_or(0))
    };
    let mut best: Option<((u32, u32), PathBuf)> = None;
    for root in roots {
        let Ok(entries) = std::fs::read_dir(root) else { continue };
        for e in entries.flatten() {
            let name = e.file_name().to_string_lossy().into_owned();
            if !(name.starts_with('v') || name.starts_with("cuda-")) {
                continue;
            }
            let cand = e.path().join("bin").join(exe);
            let v = ver(&name);
            if cand.exists() && best.as_ref().is_none_or(|(bv, _)| v > *bv) {
                best = Some((v, cand));
            }
        }
    }
    best.map(|(_, p)| p)
}

/// Locate a `vcvars64.bat` under the standard Visual Studio install dirs.
#[cfg(windows)]
fn find_vcvars() -> Option<PathBuf> {
    for root in [r"C:\Program Files\Microsoft Visual Studio", r"C:\Program Files (x86)\Microsoft Visual Studio"] {
        let Ok(years) = std::fs::read_dir(root) else { continue };
        for y in years.flatten() {
            for ed in ["BuildTools", "Community", "Professional", "Enterprise"] {
                let p = y.path().join(ed).join(r"VC\Auxiliary\Build\vcvars64.bat");
                if p.exists() {
                    return Some(p);
                }
            }
        }
    }
    None
}
