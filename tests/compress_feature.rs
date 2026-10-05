//! The coder is the one part of trex outside the default features. A binary
//! built without the `compress` feature answers every coder command with the
//! flag that builds one, rather than as an unknown command, a file name it
//! cannot read, or a coder it does not have; asked for help, it gives the
//! usage with the same flag and succeeds.

#[cfg(not(feature = "compress"))]
#[test]
fn a_build_without_the_coder_names_the_feature_that_builds_it() {
    let cases: [&[&str]; 4] = [
        &["compress", "--text", "a b c"],
        &["seam", "--text", "a b c", "--compress"],
        &["seam", "--build-model", "corpus.txt", "model.br"],
        &["seam", "--text", "a b c", "--chunks", "2"],
    ];
    for args in cases {
        let out = std::process::Command::new(env!("CARGO_BIN_EXE_trex"))
            .args(args)
            .output()
            .expect("run trex");
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert!(!out.status.success(), "{args:?} succeeded in a build without the coder");
        assert!(
            stderr.contains("cargo build --release --features compress"),
            "{args:?}: the error does not name the build that has the coder: {stderr}"
        );
    }
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_trex"))
        .args(["compress", "--help"])
        .output()
        .expect("run trex");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(out.status.success(), "compress --help failed: {}", String::from_utf8_lossy(&out.stderr));
    assert!(stdout.starts_with("usage: trex compress"), "compress --help gave no usage: {stdout}");
    assert!(
        stdout.contains("cargo build --release --features compress"),
        "compress --help does not name the build that has the coder: {stdout}"
    );
}

/// The segmentation `seam` shares a module with the coder, and it stays in
/// every build: a default binary still segments.
#[test]
fn seam_segments_in_every_build() {
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_trex"))
        .args(["seam", "--text", "the cat sat on the mat and the cat ran"])
        .output()
        .expect("run trex");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(out.status.success(), "seam failed: {}", String::from_utf8_lossy(&out.stderr));
    assert!(stdout.contains("segments"), "seam printed no segments: {stdout}");
}
