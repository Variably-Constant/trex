//! An axis command's JSON holds what its listings print: `trex shape --json`
//! carries the dominant period and every token's frame, its period strength
//! among them, as `--classes` prints each frame, and `trex seam --json` the
//! strength of a cut before each byte, as `--field` prints it.

use std::process::Command;

fn stdout(args: &[&str]) -> String {
    let out = Command::new(env!("CARGO_BIN_EXE_trex")).args(args).output().expect("run trex");
    assert!(out.status.success(), "{args:?} failed: {}", String::from_utf8_lossy(&out.stderr));
    String::from_utf8_lossy(&out.stdout).replace("\r\n", "\n")
}

#[test]
fn shape_json_holds_the_dominant_period_and_every_frame() {
    let text = "foo(a, b) bar(c, d) baz(e, f)";
    let json = stdout(&["shape", "--json", "--text", text]);
    assert!(json.starts_with("{\"n_tokens\":18,\"dominant_period\":6,\"dominant_strength\":1.0000,"), "{json}");
    // A frame a token, each keyed by the token's byte span.
    assert_eq!(json.matches("\"period_strength\":").count(), 18, "{json}");
    assert!(
        json.contains(
            "\"frames\":[{\"start\":0,\"end\":3,\"class\":65541,\"period\":0,\"period_strength\":0.0000,\"novelty\":1.0000},"
        ),
        "{json}"
    );
    // `f`, the 17th token, reads period 6 at full strength.
    assert!(
        json.contains("{\"start\":27,\"end\":28,\"class\":65541,\"period\":6,\"period_strength\":1.0000,\"novelty\":0.3333}"),
        "{json}"
    );
}

#[test]
fn shape_classes_prints_each_frames_period_strength() {
    let listing = stdout(&["shape", "--classes", "--text", "foo(a, b) bar(c, d) baz(e, f)"]);
    assert!(listing.contains("  per-token (token -> class | period | strength | novelty):\n"), "{listing}");
    assert!(listing.contains("    foo            cls=00010005 per=0   str=0.00 nov=1.00\n"), "{listing}");
    assert!(listing.contains("    f              cls=00010005 per=6   str=1.00 nov=0.33\n"), "{listing}");
}

#[test]
fn seam_json_and_field_carry_each_cuts_strength() {
    let text = "thebirdsflyoverthehouses";
    let json = stdout(&["seam", "--json", "--english", "--text", text]);
    assert!(json.starts_with("{\"len\":24,\"cuts\":[0,3,8,11,15,18],"), "{json}");
    // Under the model a cut's strength is the surprisal of the byte after it
    // plus that of the byte before it: 8.295 and 8.011 before `b`.
    assert!(json.contains("\"boundary\":[0.000,10.570,10.477,16.306,"), "{json}");
    let field = stdout(&["seam", "--field", "--english", "--limit", "4", "--text", text]);
    assert!(field.contains("  per-byte branching entropy, and the strength of a cut before the byte:\n"), "{field}");
    assert!(field.contains("         3 b  fwd=8.29 bwd=6.43 str=16.31\n"), "{field}");
}

#[test]
fn shape_json_reads_no_period_as_zero() {
    let json = stdout(&["shape", "--json", "--text", "alpha"]);
    assert_eq!(
        json.trim_end(),
        "{\"n_tokens\":1,\"dominant_period\":0,\"dominant_strength\":0.0000,\"boundaries\":[],\"regions\":[],\"frames\":[{\"start\":0,\"end\":5,\"class\":65536,\"period\":0,\"period_strength\":0.0000,\"novelty\":1.0000}]}"
    );
}
