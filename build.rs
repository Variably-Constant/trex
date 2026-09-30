//! Build script: bake the CUDA kernels into the binary.
//!
//! The `gpu` feature is on by default. When it is on and `nvcc` is
//! available, `nvcc` compiles `kernels/scan.cu` to PTX targeting the
//! `compute_75` virtual architecture (the oldest the CUDA 13 toolkit
//! emits; the driver JIT-compiles it up to the running device), and the
//! PTX path is exported so the source embeds it and the runtime loads it
//! through the driver - the deployed binary then needs only the NVIDIA
//! driver. `kernels/compress.cu`, the device coder, is baked the same way
//! only when the `compress` feature is on as well. When `nvcc` is absent
//! this writes a placeholder instead of failing, so a default build still
//! succeeds and runs CPU-only. With the feature off
//! (`--no-default-features`) this is a no-op and the crate builds as pure
//! Rust with no CUDA involvement.

use std::path::{Path, PathBuf};
use std::process::Command;

fn main() {
    println!("cargo:rerun-if-changed=kernels/scan.cu");
    println!("cargo:rerun-if-changed=kernels/compress.cu");
    println!("cargo:rerun-if-changed=build.rs");

    // Pure-CPU build: the gpu feature is off (--no-default-features), so
    // there is no kernel to compile.
    if std::env::var_os("CARGO_FEATURE_GPU").is_none() {
        return;
    }

    let out_dir = PathBuf::from(std::env::var("OUT_DIR").expect("OUT_DIR set by cargo"));

    // gpu is on by default. Bake each kernel; if that fails for any reason -
    // nvcc absent, or present but unable to run (no cl.exe on Windows without
    // vcvars) - write a placeholder so the crate still builds and runs CPU-only.
    // The runtime's module load rejects the placeholder, so the device path is
    // simply unavailable. A default build never fails for lack of a toolchain.
    let mut warned = false;
    let mut kernels = vec![("kernels/scan.cu", "scan.ptx", "TREX_SCAN_PTX")];
    if std::env::var_os("CARGO_FEATURE_COMPRESS").is_some() {
        kernels.push(("kernels/compress.cu", "compress.ptx", "TREX_COMPRESS_PTX"));
    }
    for (src, out_name, env_var) in kernels {
        let ptx_path = out_dir.join(out_name);
        if !bake_ptx(src, &ptx_path) {
            std::fs::write(&ptx_path, "// trex: no CUDA kernel baked at build time\n")
                .expect("write placeholder PTX to OUT_DIR");
            if !warned {
                println!(
                    "cargo:warning=trex: could not bake the CUDA kernels (nvcc missing or \
                     failed; on Windows build from a shell with vcvars so nvcc finds cl.exe). \
                     The binary is CPU-only. Build with --no-default-features for pure CPU."
                );
                warned = true;
            }
        }
        println!("cargo:rustc-env={env_var}={}", ptx_path.display());
    }
}

/// Compile the CUDA source `src` to PTX at `ptx_path` via `nvcc`. Returns
/// `true` only when a PTX file was produced; any failure (nvcc missing,
/// nvcc erroring, no `cl.exe` on Windows) returns `false` so the caller
/// falls back to a CPU-only build.
fn bake_ptx(src: &str, ptx_path: &Path) -> bool {
    let Some(nvcc) = find_nvcc() else {
        return false;
    };
    let ptx = ptx_path.to_string_lossy().into_owned();
    let direct = Command::new(&nvcc).args(["-ptx", "-arch=compute_75", "-o", ptx.as_str(), src]).status();
    if matches!(direct, Ok(s) if s.success()) && ptx_path.exists() {
        return true;
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
        if std::fs::write(&bat, script).is_ok() {
            let ran = Command::new("cmd").arg("/C").arg(&bat).status();
            return matches!(ran, Ok(s) if s.success()) && ptx_path.exists();
        }
    }
    false
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
