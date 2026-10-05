//! The marks of a marked example: `{name:text}` around the text a field
//! holds, read as Windows PowerShell's `ConvertFrom-String` reads a template,
//! so a template saved for it reads the same here.
//!
//! A mark opens with `{`, an optional `[type]` hint, the field's name, an
//! optional `*` saying the field begins a record, and `:`. Everything up to
//! the `}` that closes it is the field's text, and that text may hold marks
//! of its own, which name the parts of the field. `\{`, `\}` and `\\` are a
//! literal brace and a literal backslash, and a backslash before anything
//! else is itself, so a Windows path reads as written. A `{` that opens no
//! mark and a `}` that closes none are refused rather than read as text,
//! since `ConvertFrom-String` reads neither as text either.

use crate::token::TokenKind;

/// A marked example with its marks taken out.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Marked {
    /// The example as it reads: every mark taken out and every escape
    /// resolved.
    pub text: String,
    /// The marks in the order they open, so a mark inside another follows
    /// the one holding it.
    pub marks: Vec<Mark>,
}

/// One `{name:text}` mark.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Mark {
    /// The field the mark names.
    pub name: String,
    /// The field's text in [`Marked::text`], as a byte range.
    pub span: std::ops::Range<usize>,
    /// The type a `{[type]name:text}` mark names.
    pub hint: Option<Hint>,
    /// The type's name as the mark writes it, `int` or `System.Int32`, which
    /// is what a value is cast to.
    pub type_name: Option<String>,
    /// `{name*:text}`: the field begins a record.
    pub starts_record: bool,
    /// The mark that encloses this one, by its index in [`Marked::marks`].
    pub parent: Option<usize>,
}

/// The type a mark's `[type]` names, which fixes the kind of token the
/// field's text is read as and the type its value is reported in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Hint {
    /// A whole number: `int`, `long`, `short`, `byte`, their unsigned and
    /// sized forms, and `bigint`.
    Integer,
    /// A number with or without a fraction: `double`, `float`, `single`,
    /// `decimal`.
    Number,
    /// `bool`: true or false.
    Boolean,
    /// `char`: one character.
    Char,
    /// `string`: any text, which constrains nothing.
    Text,
    /// `datetime` and `datetimeoffset`.
    Timestamp,
    /// `timespan`.
    Duration,
    /// `guid`.
    Uuid,
    /// `version`.
    Version,
    /// `ipaddress`.
    Ip,
    /// `mailaddress`.
    Email,
    /// `uri`.
    Url,
}

impl Hint {
    /// The hint a `[type]` names: a PowerShell type accelerator or .NET type
    /// name, in any case, with or without its `System.` or
    /// `System.Net.` namespace.
    #[must_use]
    pub fn parse(name: &str) -> Option<Hint> {
        let lower = name.trim().to_ascii_lowercase();
        let bare = lower
            .strip_prefix("system.net.mail.")
            .or_else(|| lower.strip_prefix("system.net."))
            .or_else(|| lower.strip_prefix("system.numerics."))
            .or_else(|| lower.strip_prefix("system."))
            .unwrap_or(&lower);
        Some(match bare {
            "int" | "int16" | "int32" | "int64" | "long" | "short" | "byte" | "sbyte" | "uint" | "uint16"
            | "uint32" | "uint64" | "ulong" | "ushort" | "bigint" | "biginteger" => Hint::Integer,
            "double" | "float" | "single" | "decimal" => Hint::Number,
            "bool" | "boolean" => Hint::Boolean,
            "char" => Hint::Char,
            "string" => Hint::Text,
            "datetime" | "datetimeoffset" => Hint::Timestamp,
            "timespan" => Hint::Duration,
            "guid" => Hint::Uuid,
            "version" => Hint::Version,
            "ipaddress" => Hint::Ip,
            "mailaddress" => Hint::Email,
            "uri" => Hint::Url,
            _ => return None,
        })
    }

    /// The type as a mark names it.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Hint::Integer => "int",
            Hint::Number => "double",
            Hint::Boolean => "bool",
            Hint::Char => "char",
            Hint::Text => "string",
            Hint::Timestamp => "datetime",
            Hint::Duration => "timespan",
            Hint::Uuid => "guid",
            Hint::Version => "version",
            Hint::Ip => "ipaddress",
            Hint::Email => "mailaddress",
            Hint::Url => "uri",
        }
    }

    /// The kind of token a field of this type is read as; `None` for a type
    /// that fixes no kind, which a string, a character and a boolean are.
    #[must_use]
    pub fn kind(self) -> Option<TokenKind> {
        match self {
            Hint::Integer | Hint::Number => Some(TokenKind::Number),
            Hint::Timestamp => Some(TokenKind::Timestamp),
            Hint::Duration => Some(TokenKind::Duration),
            Hint::Uuid => Some(TokenKind::Uuid),
            Hint::Version => Some(TokenKind::Version),
            Hint::Ip => Some(TokenKind::Ip),
            Hint::Email => Some(TokenKind::Email),
            Hint::Url => Some(TokenKind::Url),
            Hint::Boolean | Hint::Char | Hint::Text => None,
        }
    }
}

/// Why a marked example does not read.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MarkError {
    /// The column of the fault, from one, counted in characters.
    pub col: usize,
    /// What is wrong there and how to write it instead.
    pub msg: String,
}

impl std::fmt::Display for MarkError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "column {}: {}", self.col, self.msg)
    }
}

/// A mark's header, `[type]name*:`, read from just after its `{`: the hint
/// with the type's name as written, the name, whether it begins a record,
/// and the byte length of the header.
type Header = (Option<(Hint, String)>, String, bool, usize);

/// The header of a mark opening at the start of `rest`, the text just after a
/// `{`; `Ok(None)` where `rest` opens no header at all, and an error where it
/// opens one and gets it wrong, which is a `[type]` naming no known type. The
/// brackets are a type only where what they hold reads as a type's name, so
/// `{[1,2]}` opens no mark rather than naming a type.
fn header(rest: &str) -> Result<Option<Header>, String> {
    let bytes = rest.as_bytes();
    let mut i = 0;
    let mut hint = None;
    if bytes.first() == Some(&b'[') {
        let Some(close) = rest.find(']') else { return Ok(None) };
        let name = &rest[1..close];
        let typelike = name.starts_with(|c: char| c.is_ascii_alphabetic())
            && name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'.');
        if !typelike {
            return Ok(None);
        }
        match Hint::parse(name) {
            Some(h) => hint = Some((h, name.to_string())),
            None => {
                return Err(format!(
                    "[{name}] names no type a mark knows; write one of int, long, double, decimal, bool, char, \
                     string, datetime, timespan, guid, version, ipaddress, mailaddress or uri"
                ));
            }
        }
        i = close + 1;
    }
    let start = i;
    if !matches!(bytes.get(i), Some(&c) if c == b'_' || c.is_ascii_alphabetic()) {
        return Ok(None);
    }
    while matches!(bytes.get(i), Some(&c) if c == b'_' || c.is_ascii_alphanumeric()) {
        i += 1;
    }
    let name = rest[start..i].to_string();
    let starts_record = bytes.get(i) == Some(&b'*');
    if starts_record {
        i += 1;
    }
    if bytes.get(i) != Some(&b':') {
        return Ok(None);
    }
    Ok(Some((hint, name, starts_record, i + 1)))
}

/// The column, from one and in characters, of byte `at` of `src`.
fn column(src: &str, at: usize) -> usize {
    src[..at].chars().count() + 1
}

/// One field of a built pattern as a `fields` line of a pattern file writes
/// it: a mark with the example text left out.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FieldMark {
    /// The field's name; a field inside another is named after it, a dot,
    /// and its own name, as TREX names a capture inside a capture.
    pub name: String,
    /// The type a `{[type]name}` mark names.
    pub hint: Option<Hint>,
    /// The type's name as the mark writes it, which is what a value is cast
    /// to.
    pub type_name: Option<String>,
    /// `{name*}`: the field begins a record.
    pub starts_record: bool,
    /// `{name:accessor}`: the field reads its register through this
    /// accessor, as `${name:accessor}` does in a template.
    pub accessor: Option<String>,
}

impl FieldMark {
    /// The mark a `fields` line writes for the field.
    #[must_use]
    pub fn written(&self) -> String {
        let mut out = String::from("{");
        if let Some(t) = &self.type_name {
            out.push('[');
            out.push_str(t);
            out.push(']');
        }
        out.push_str(&self.name);
        if self.starts_record {
            out.push('*');
        }
        if let Some(a) = &self.accessor {
            out.push(':');
            out.push_str(a);
        }
        out.push('}');
        out
    }
}

/// Read the fields of a `fields` line: marks with the example text left
/// out, `{[type]name*:accessor}`, each part but the name optional, separated
/// by whitespace. A name is alphanumerics and underscores, dots joining a
/// field inside another to it; an accessor runs to the `}`.
///
/// # Errors
///
/// Anything between the marks, a mark never closed, a mark with no name or
/// an empty accessor, and a `[type]` naming no type a mark knows, each with
/// its column.
pub fn field_marks(src: &str) -> Result<Vec<FieldMark>, MarkError> {
    let mut out = Vec::new();
    let mut i = 0;
    let bytes = src.as_bytes();
    while i < bytes.len() {
        if bytes[i].is_ascii_whitespace() {
            i += 1;
            continue;
        }
        let err = |at: usize, msg: String| MarkError { col: column(src, at), msg };
        if bytes[i] != b'{' {
            return Err(err(i, "a fields line holds marks, {name}, {[type]name}, {name*} or {name:accessor}".to_string()));
        }
        let open = i;
        let Some(close) = src[open..].find('}').map(|c| open + c) else {
            return Err(err(open, "the mark is never closed with a }".to_string()));
        };
        let body = &src[open + 1..close];
        let (typed, rest) = match body.strip_prefix('[') {
            Some(after) => {
                let Some((name, rest)) = after.split_once(']') else {
                    return Err(err(open, "the [type] is never closed with a ]".to_string()));
                };
                match Hint::parse(name) {
                    Some(h) => (Some((h, name.to_string())), rest),
                    None => {
                        return Err(err(
                            open,
                            format!(
                                "[{name}] names no type a mark knows; write one of int, long, double, decimal, bool, \
                                 char, string, datetime, timespan, guid, version, ipaddress, mailaddress or uri"
                            ),
                        ));
                    }
                }
            }
            None => (None, body),
        };
        let (head, accessor) = match rest.split_once(':') {
            Some((head, accessor)) => (head, Some(accessor.trim().to_string())),
            None => (rest, None),
        };
        let (name, starts_record) = match head.strip_suffix('*') {
            Some(name) => (name, true),
            None => (head, false),
        };
        let named = !name.is_empty()
            && name.split('.').all(|part| {
                part.starts_with(|c: char| c == '_' || c.is_ascii_alphabetic())
                    && part.chars().all(|c| c == '_' || c.is_ascii_alphanumeric())
            });
        if !named {
            return Err(err(
                open,
                format!("{{{body}}} names no field; a name is alphanumerics and underscores, dots joining a field inside another"),
            ));
        }
        if accessor.as_deref().is_some_and(str::is_empty) {
            return Err(err(open, format!("{{{body}}} names no accessor after its colon")));
        }
        let (hint, type_name) = match typed {
            Some((h, written)) => (Some(h), Some(written)),
            None => (None, None),
        };
        out.push(FieldMark { name: name.to_string(), hint, type_name, starts_record, accessor });
        i = close + 1;
    }
    Ok(out)
}

/// Read a marked example.
///
/// # Errors
///
/// A `{` that opens no mark, a `}` that closes none, a mark never closed, a
/// mark holding no text, and a `[type]` naming no type a mark knows, each
/// with its column.
pub fn parse(src: &str) -> Result<Marked, MarkError> {
    scan(src).map(|(marked, _)| marked)
}

/// [`parse`], with the byte offset in `src` of each mark's `{`.
fn scan(src: &str) -> Result<(Marked, Vec<usize>), MarkError> {
    let mut opens: Vec<usize> = Vec::new();
    let mut text = String::with_capacity(src.len());
    let mut marks: Vec<Mark> = Vec::new();
    // The marks open at this point, innermost last, each with the column of
    // its `{` for a mark never closed.
    let mut open: Vec<(usize, usize)> = Vec::new();
    let mut i = 0;
    while i < src.len() {
        let rest = &src[i..];
        let c = rest.chars().next().expect("a character at every position below the end");
        match c {
            '\\' => match rest[1..].chars().next() {
                Some(escaped @ ('{' | '}' | '\\')) => {
                    text.push(escaped);
                    i += 2;
                }
                _ => {
                    text.push('\\');
                    i += 1;
                }
            },
            '{' => match header(&rest[1..]) {
                Ok(Some((typed, name, starts_record, len))) => {
                    let parent = open.last().map(|&(m, _)| m);
                    let (hint, type_name) = match typed {
                        Some((hint, written)) => (Some(hint), Some(written)),
                        None => (None, None),
                    };
                    let span = text.len()..text.len();
                    marks.push(Mark { name, span, hint, type_name, starts_record, parent });
                    opens.push(i);
                    open.push((marks.len() - 1, column(src, i)));
                    i += 1 + len;
                }
                Ok(None) => {
                    return Err(MarkError {
                        col: column(src, i),
                        msg: "a { that opens no mark; a mark is {name:text}, and \\{ writes a literal brace"
                            .to_string(),
                    });
                }
                Err(msg) => return Err(MarkError { col: column(src, i), msg }),
            },
            '}' => {
                let Some((m, col)) = open.pop() else {
                    return Err(MarkError {
                        col: column(src, i),
                        msg: "a } that closes no mark; \\} writes a literal brace".to_string(),
                    });
                };
                marks[m].span.end = text.len();
                if marks[m].span.is_empty() {
                    return Err(MarkError {
                        col,
                        msg: format!("the mark {{{}:}} holds no text, so nothing places the field", marks[m].name),
                    });
                }
                i += 1;
            }
            other => {
                text.push(other);
                i += other.len_utf8();
            }
        }
    }
    if let Some(&(m, col)) = open.last() {
        return Err(MarkError { col, msg: format!("the mark {{{}:...}} is never closed with a }}", marks[m].name) });
    }
    Ok((Marked { text, marks }, opens))
}

/// Read a template of marked lines, as `ConvertFrom-String` reads one given
/// as a single text: each line a marked example, a blank line left out, a
/// carriage return before a newline left off. A mark spanning lines holds
/// on each line it covers the part of it that line holds, the marks inside
/// it nested under it there, and where it is starred the record it begins
/// begins at the first mark inside it, so the lines it spans are one record.
///
/// # Errors
///
/// As [`parse`], with the line of the fault, counted from zero among
/// all the lines, and the column on that line; and a starred mark spanning
/// lines that holds no mark on its first line, since nothing there says
/// which line of the input begins its record.
pub fn parse_lines(src: &str) -> Result<Vec<Marked>, (usize, MarkError)> {
    let place = |at: usize, msg: String| {
        let before = &src[..at];
        let col = before.rsplit('\n').next().expect("a split yields a part").chars().count() + 1;
        (before.matches('\n').count(), MarkError { col, msg })
    };
    let (mut whole, opens) = scan(src).map_err(|e| {
        let (at, _) = src.char_indices().nth(e.col - 1).expect("a fault is at a brace of the template");
        place(at, e.msg)
    })?;
    for (k, &open) in opens.iter().enumerate() {
        let span = whole.marks[k].span.clone();
        if !(whole.marks[k].starts_record && whole.text[span.clone()].contains('\n')) {
            continue;
        }
        let first = (k + 1..whole.marks.len()).find(|&c| whole.marks[c].parent == Some(k));
        let on_first_line = first.is_some_and(|c| {
            let before = &whole.text[span.start..whole.marks[c].span.start];
            before.rsplit_once('\n').is_none_or(|(lines, _)| lines.trim().is_empty())
        });
        let Some(first) = first.filter(|_| on_first_line) else {
            let msg = format!(
                "the starred mark {{{}*:...}} spans lines, so its record begins at a mark inside it on its first line, \
                 and it holds none there",
                whole.marks[k].name
            );
            return Err(place(open, msg));
        };
        whole.marks[k].starts_record = false;
        whole.marks[first].starts_record = true;
    }
    let mut out = Vec::new();
    let mut start = 0;
    for piece in whole.text.split('\n') {
        let body = piece.strip_suffix('\r').unwrap_or(piece);
        if !body.trim().is_empty() {
            out.push(line_of(&whole, start..start + body.len()));
        }
        start += piece.len() + 1;
    }
    Ok(out)
}

/// The line of `whole` that bytes `range` of its text hold, as a marked
/// example of its own: each mark holding text there, clipped to the line,
/// a mark inside another nested under that one's part of the line.
fn line_of(whole: &Marked, range: std::ops::Range<usize>) -> Marked {
    let mut marks: Vec<Mark> = Vec::new();
    let mut here: Vec<Option<usize>> = vec![None; whole.marks.len()];
    for (k, m) in whole.marks.iter().enumerate() {
        let span = m.span.start.max(range.start)..m.span.end.min(range.end);
        if span.start >= span.end || whole.text[span.clone()].trim().is_empty() {
            continue;
        }
        let mut mark = m.clone();
        mark.span = span.start - range.start..span.end - range.start;
        mark.parent = m.parent.and_then(|p| here[p]);
        here[k] = Some(marks.len());
        marks.push(mark);
    }
    Marked { text: whole.text[range].to_string(), marks }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The marks of `src` as `(name, text)` pairs, in the order they open.
    fn fields(src: &str) -> Vec<(String, String)> {
        let marked = parse(src).expect("the example reads");
        marked.marks.iter().map(|m| (m.name.clone(), marked.text[m.span.clone()].to_string())).collect()
    }

    #[test]
    fn a_mark_names_the_text_it_surrounds_and_leaves_the_line_as_it_reads() {
        let marked = parse("2023-10 Update for Windows 11 ({kb:KB5031354})").expect("reads");
        assert_eq!(marked.text, "2023-10 Update for Windows 11 (KB5031354)");
        assert_eq!(fields("2023-10 Update for Windows 11 ({kb:KB5031354})"), [("kb".to_string(), "KB5031354".to_string())]);
        assert_eq!(marked.marks[0].span, 31..40);
        assert_eq!(
            fields("{date:2023-10} Update for {os:Windows 11} ({kb:KB5031354})"),
            [
                ("date".to_string(), "2023-10".to_string()),
                ("os".to_string(), "Windows 11".to_string()),
                ("kb".to_string(), "KB5031354".to_string())
            ]
        );
    }

    #[test]
    fn a_mark_inside_a_mark_names_a_part_of_its_field() {
        let marked = parse("set {pair:{k:x} = {v:1}} now").expect("reads");
        assert_eq!(marked.text, "set x = 1 now");
        assert_eq!(marked.marks.len(), 3);
        assert_eq!(marked.marks[0].parent, None);
        assert_eq!(marked.marks[1].parent, Some(0));
        assert_eq!(marked.marks[2].parent, Some(0));
        assert_eq!(&marked.text[marked.marks[0].span.clone()], "x = 1");
    }

    #[test]
    fn convertfrom_string_markup_reads_unchanged() {
        // The template from Microsoft's own ConvertFrom-String page.
        let marked = parse("{[string]Name*:Phoebe Cat}, {phone:425-123-6789}, {[int]age:6}").expect("reads");
        assert_eq!(marked.text, "Phoebe Cat, 425-123-6789, 6");
        assert_eq!(marked.marks[0].hint, Some(Hint::Text));
        assert!(marked.marks[0].starts_record);
        assert!(!marked.marks[1].starts_record);
        assert_eq!(marked.marks[2].hint, Some(Hint::Integer));
        assert_eq!(Hint::parse("System.Int32"), Some(Hint::Integer));
        assert_eq!(Hint::parse("IPAddress"), Some(Hint::Ip));
        assert_eq!(Hint::parse("System.Net.Mail.MailAddress"), Some(Hint::Email));
        assert_eq!(Hint::Integer.kind(), Some(TokenKind::Number));
        assert_eq!(Hint::Text.kind(), None);
    }

    /// The escapes are ConvertFrom-String's, measured against Windows
    /// PowerShell 5.1: `\{`, `\}` and `\\` are literal, and any other
    /// backslash is itself.
    #[test]
    fn a_backslash_escapes_a_brace_or_a_backslash_and_nothing_else() {
        assert_eq!(parse(r"x \{ {n:1} \} y").expect("reads").text, "x { 1 } y");
        assert_eq!(fields(r"x \{ {n:1} \} y"), [("n".to_string(), "1".to_string())]);
        assert_eq!(parse(r"open C:\Windows\a.txt").expect("reads").text, r"open C:\Windows\a.txt");
        assert_eq!(fields(r"open C:\\Windows\\{f:a.txt}"), [("f".to_string(), "a.txt".to_string())]);
        assert_eq!(parse(r"open C:\\Windows\\{f:a.txt}").expect("reads").text, r"open C:\Windows\a.txt");
    }

    #[test]
    fn a_brace_that_opens_or_closes_no_mark_is_refused_with_its_column() {
        let open = parse(r#"log {"level":"info"}"#).expect_err("a bare brace");
        assert_eq!(open.col, 5);
        assert!(open.msg.contains(r"\{"), "{}", open.msg);
        let close = parse("a } b").expect_err("a stray close");
        assert_eq!(close.col, 3);
        let never = parse("a {x:b").expect_err("never closed");
        assert_eq!(never.col, 3);
        assert!(never.msg.contains("never closed"), "{}", never.msg);
        let empty = parse("a {x:} b").expect_err("empty");
        assert!(empty.msg.contains("holds no text"), "{}", empty.msg);
        let unknown = parse("a {[widget]x:1} b").expect_err("an unknown type");
        assert!(unknown.msg.contains("[widget]"), "{}", unknown.msg);
        let json = parse("list {[1,2]} end").expect_err("brackets holding no type");
        assert!(json.msg.contains("opens no mark"), "{}", json.msg);
    }

    #[test]
    fn a_mark_spanning_lines_holds_its_part_of_each_and_its_record_begins_at_its_first_inner_mark() {
        let lines = parse_lines("{Person*:Name: {Name:Phoebe Cat}\r\n\r\nPhone: {Phone:425-123-6789}}\n").expect("reads");
        let texts: Vec<&str> = lines.iter().map(|l| l.text.as_str()).collect();
        assert_eq!(texts, ["Name: Phoebe Cat", "Phone: 425-123-6789"]);
        let read = |l: &Marked| -> Vec<(String, String, Option<usize>, bool)> {
            l.marks.iter().map(|m| (m.name.clone(), l.text[m.span.clone()].to_string(), m.parent, m.starts_record)).collect()
        };
        let mark = |name: &str, text: &str, parent: Option<usize>, starts: bool| (name.to_string(), text.to_string(), parent, starts);
        assert_eq!(read(&lines[0]), [mark("Person", "Name: Phoebe Cat", None, false), mark("Name", "Phoebe Cat", Some(0), true)]);
        assert_eq!(
            read(&lines[1]),
            [mark("Person", "Phone: 425-123-6789", None, false), mark("Phone", "425-123-6789", Some(0), false)]
        );
        let plain = parse_lines("a {x:1\n2} b").expect("reads");
        assert_eq!(read(&plain[0]), [mark("x", "1", None, false)]);
        assert_eq!(read(&plain[1]), [mark("x", "2", None, false)]);
    }

    #[test]
    fn a_fault_in_a_template_of_lines_names_its_line_and_its_column_there() {
        let (line, e) = parse_lines("a {x:1}\nb } c").expect_err("a stray close");
        assert_eq!((line, e.col), (1, 3));
        let (line, e) = parse_lines("a\n\nb {x:1\nc").expect_err("never closed");
        assert_eq!((line, e.col), (2, 3));
    }

    #[test]
    fn a_starred_mark_spanning_lines_needs_a_mark_on_its_first_line_to_begin_its_record() {
        let (line, e) = parse_lines("{Addr*:12 Main St\nSpringfield}").expect_err("no mark inside");
        assert_eq!((line, e.col), (0, 1));
        assert!(e.msg.contains("{Addr*:...} spans lines"), "{}", e.msg);
        let (line, e) = parse_lines("x\nsee {P*:head\n{Name:a}}").expect_err("no mark on its first line");
        assert_eq!((line, e.col), (1, 5));
        let lines = parse_lines("{P*:\n{Name:a}\n{Phone:b}}").expect("the mark's first line holding text holds Name");
        assert_eq!(lines.len(), 2);
        assert!(lines[0].marks[1].starts_record && !lines[0].marks[0].starts_record);
        assert!(!lines[1].marks[1].starts_record);
    }

    #[test]
    fn a_column_counts_characters_not_bytes() {
        let err = parse("café } x").expect_err("a stray close after a two-byte letter");
        assert_eq!(err.col, 6);
    }
}
