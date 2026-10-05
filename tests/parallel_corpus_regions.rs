//! The region classification over the parallel corpus: the Universal
//! Declaration of Human Rights in sixteen languages reads as prose in every
//! script, and the same bytes encoded read as a blob, wrapped as MIME wraps
//! base64 or listed as SHA-256 digests one to a line.
//!
//! `_corpus/parallel/SOURCES.md` names the texts. The corpus is in the
//! repository and not the published crate, as these tests are.

use std::fs;
use std::path::{Path, PathBuf};

use trex::shape::{RegionKind, dominant_kind};

const CORPUS: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/_corpus/parallel");

fn translations() -> Vec<PathBuf> {
    let dir = Path::new(CORPUS).join("prose");
    let mut paths: Vec<PathBuf> = fs::read_dir(&dir)
        .unwrap_or_else(|e| panic!("{}: {e}", dir.display()))
        .map(|e| e.unwrap_or_else(|e| panic!("{}: {e}", dir.display())).path())
        .collect();
    paths.sort();
    assert_eq!(paths.len(), 16, "the sixteen translations SOURCES.md lists: {paths:?}");
    paths
}

fn read(path: &Path) -> Vec<u8> {
    fs::read(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

#[test]
fn every_translation_reads_as_prose() {
    for path in translations() {
        assert_eq!(dominant_kind(&read(&path)), Some(RegionKind::Prose), "{}", path.display());
    }
}

#[test]
fn a_region_preview_holds_whole_characters() {
    for path in translations() {
        let out = std::process::Command::new(env!("CARGO_BIN_EXE_trex"))
            .args(["spectral", "--classify"])
            .arg(&path)
            .output()
            .expect("run trex");
        assert!(out.status.success(), "{}: {}", path.display(), String::from_utf8_lossy(&out.stderr));
        let text = String::from_utf8(out.stdout).expect("the listing is UTF-8");
        assert!(!text.contains('\u{FFFD}'), "{}: {text}", path.display());
    }
}

/// `data` in base64, 76 characters to a line.
fn base64_lines(data: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut flat = String::new();
    for chunk in data.chunks(3) {
        let n = chunk.iter().enumerate().fold(0u32, |a, (i, &b)| a | u32::from(b) << (16 - 8 * i));
        for k in 0..=chunk.len() {
            flat.push(char::from(ALPHABET[(n >> (18 - 6 * k) & 63) as usize]));
        }
        flat.extend(std::iter::repeat_n('=', 3 - chunk.len()));
    }
    flat.as_bytes().chunks(76).map(|l| std::str::from_utf8(l).expect("base64 is ASCII")).collect::<Vec<_>>().join("\n")
}

#[test]
fn the_translations_in_base64_read_as_a_blob() {
    let all: Vec<u8> = translations().iter().flat_map(|p| read(p)).collect();
    let encoded = base64_lines(&all);
    assert_eq!(dominant_kind(encoded.as_bytes()), Some(RegionKind::Blob));
}

#[test]
fn the_manifests_digests_read_as_a_blob() {
    let manifest = read(&Path::new(CORPUS).join("MANIFEST.sha256"));
    let text = std::str::from_utf8(&manifest).expect("the manifest is UTF-8");
    let digests: Vec<&str> = text
        .lines()
        .map(|l| match l.split_whitespace().next() {
            Some(d) if d.len() == 64 && d.bytes().all(|b| b.is_ascii_hexdigit()) => d,
            _ => panic!("MANIFEST.sha256: no digest on {l:?}"),
        })
        .collect();
    assert!(digests.len() > 100, "{} digests", digests.len());
    assert_eq!(dominant_kind(digests.join("\n").as_bytes()), Some(RegionKind::Blob));
}
