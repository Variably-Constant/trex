//! A pattern file or a word set in any UTF encoding its byte-order mark
//! declares reads as its text.
//!
//! Windows PowerShell writes a file with `Out-File` or `>` as UTF-16 LE behind
//! a BOM, and with `-Encoding UTF8` as UTF-8 behind one, so a declaration file
//! saved from PowerShell arrives in one of those shapes. Read as raw UTF-8, the
//! first reads its whole text as noise and the second opens its first line with
//! a character no declaration keyword starts with.

use trex::ShapeSet;

/// A scratch directory of its own, removed when the test ends.
struct Dir(std::path::PathBuf);

impl Dir {
    fn new(tag: &str) -> Dir {
        let dir = std::env::temp_dir().join(format!("trex/encodings-{tag}-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("create temp dir");
        Dir(dir)
    }

    fn write(&self, name: &str, bytes: &[u8]) -> std::path::PathBuf {
        let path = self.0.join(name);
        std::fs::write(&path, bytes).expect("write the file");
        path
    }
}

impl Drop for Dir {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.0).expect("remove temp dir");
    }
}

fn utf8_bom(text: &str) -> Vec<u8> {
    let mut bytes = vec![0xEF, 0xBB, 0xBF];
    bytes.extend_from_slice(text.as_bytes());
    bytes
}

fn utf16le_bom(text: &str) -> Vec<u8> {
    let mut bytes = vec![0xFF, 0xFE];
    for unit in text.encode_utf16() {
        bytes.extend_from_slice(&unit.to_le_bytes());
    }
    bytes
}

const DEFS: &str = "let rhs = \\N | \\Q\r\nkind assign = \\W \"=\" \\{rhs}\r\n";
const INPUT: &[u8] = b"let x = 1; name = \"bob\"";

/// The spans `\{assign}` finds in [`INPUT`] under the declarations in `path`.
fn assigns_under(path: &std::path::Path) -> Vec<(usize, usize)> {
    let mut set = ShapeSet::new();
    set.declare_file(path).expect("the file declares");
    let pattern = trex::parser::parse_with_shapes("\\{assign}", &set).expect("the pattern parses");
    trex::scan_with_shapes(&pattern, INPUT, &set).iter().map(|s| (s.start(), s.end())).collect()
}

#[test]
fn a_pattern_file_behind_a_utf8_bom_declares_its_first_line() {
    let dir = Dir::new("utf8");
    let plain = dir.write("plain.trex", DEFS.as_bytes());
    let marked = dir.write("marked.trex", &utf8_bom(DEFS));
    let expected = assigns_under(&plain);
    assert_eq!(expected, vec![(4, 9), (11, 23)]);
    assert_eq!(assigns_under(&marked), expected);
}

#[test]
fn a_pattern_file_in_utf16le_declares_what_its_text_says() {
    let dir = Dir::new("utf16");
    let marked = dir.write("wide.trex", &utf16le_bom(DEFS));
    assert_eq!(assigns_under(&marked), vec![(4, 9), (11, 23)]);
}

#[test]
fn a_file_of_members_behind_a_bom_names_the_same_members() {
    let dir = Dir::new("members");
    let text = "let greeting = \"hello\"\r\n\\W \"world\"\r\n";
    let plain = dir.write("plain.trex", text.as_bytes());
    let wide = dir.write("wide.trex", &utf16le_bom(text));
    let names = |path: &std::path::Path| -> Vec<String> {
        let mut set = ShapeSet::new();
        let members = set.declare_file_members(path).expect("the file declares");
        members.into_iter().map(|(name, _)| name).collect()
    };
    assert_eq!(names(&plain), vec!["greeting".to_string(), "2".to_string()]);
    assert_eq!(names(&wide), names(&plain));
}

#[test]
fn a_word_set_in_utf16le_reads_its_members() {
    let dir = Dir::new("set");
    let words = dir.write("words.txt", &utf16le_bom("apple\r\nbanana\r\n"));
    // Forward slashes, which every platform's file API takes, so the path
    // inside the braces holds no backslash for the predicate to read.
    let source = format!("\\W{{in:@{}}}", words.display().to_string().replace('\\', "/"));
    let pattern = trex::parse(&source).expect("the pattern parses");
    let found: Vec<(usize, usize)> =
        trex::scan(&pattern, b"an apple, a pear, a banana").iter().map(|s| (s.start(), s.end())).collect();
    assert_eq!(found, vec![(3, 8), (20, 26)]);
}
