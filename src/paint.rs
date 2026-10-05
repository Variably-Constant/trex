//! Color for the reports: the depth a console renders, a color or a style
//! sent at that depth, the roles a report paints, and the overrides
//! `--colors` spells.
//!
//! A color is held as the console's own base color, an entry of the
//! 256-color table, or a 24-bit value, and sent at the depth the console
//! renders: a 24-bit value goes to the nearest entry of the 256-color cube
//! and grays on a 256-color console, and to the nearest of the sixteen base
//! colors on a 16-color one. The base colors are sent as the base codes at
//! every depth, so the console's own theme decides their hue.

use std::io::IsTerminal;

use crate::token::{Token, TokenKind};

/// How many colors a console renders.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Depth {
    /// None: the output is not a console, or asks for no color.
    Off,
    /// The sixteen base colors.
    Ansi16,
    /// The 256-color table.
    Ansi256,
    /// Any 24-bit value.
    TrueColor,
}

impl Depth {
    /// The depth the standard output renders, read from the environment and
    /// from whether it is a terminal, with a Windows console's escape-code
    /// processing switched on where it is not already.
    #[must_use]
    pub fn detect() -> Depth {
        let is_terminal = std::io::stdout().is_terminal();
        let depth = Depth::of(is_terminal, &|name| std::env::var(name).ok());
        if depth != Depth::Off && cfg!(windows) && !enable_vt() {
            return Depth::Off;
        }
        depth
    }

    /// The depth a console renders, from the variables `env` answers and
    /// whether the output is a terminal: none where `NO_COLOR` is set, the
    /// output is not a terminal or `TERM` is `dumb`; 24-bit where `COLORTERM`
    /// is `truecolor` or `24bit`, under Windows Terminal (`WT_SESSION`), VS
    /// Code (`TERM_PROGRAM`), kitty or alacritty (`TERM`); 256 where `TERM`
    /// says `256color`; a Windows console otherwise renders 24-bit, and any
    /// other terminal sixteen.
    pub fn of(is_terminal: bool, env: &dyn Fn(&str) -> Option<String>) -> Depth {
        let set = |name: &str| env(name).is_some_and(|v| !v.is_empty());
        if set("NO_COLOR") || !is_terminal {
            return Depth::Off;
        }
        let term = env("TERM").unwrap_or_default();
        if term == "dumb" {
            return Depth::Off;
        }
        let colorterm = env("COLORTERM").unwrap_or_default();
        if colorterm == "truecolor"
            || colorterm == "24bit"
            || set("WT_SESSION")
            || env("TERM_PROGRAM").is_some_and(|p| p == "vscode")
            || term.starts_with("xterm-kitty")
            || term.starts_with("alacritty")
        {
            return Depth::TrueColor;
        }
        if term.contains("256color") {
            return Depth::Ansi256;
        }
        if cfg!(windows) && term.is_empty() {
            return Depth::TrueColor;
        }
        Depth::Ansi16
    }

    /// The depth `--color always` sends at: what the environment says with
    /// the output taken for a terminal, and sixteen where it says nothing
    /// or asks for none; a Windows console has its escape-code processing
    /// switched on.
    #[must_use]
    pub fn forced() -> Depth {
        console_ready();
        Depth::of(true, &|name| std::env::var(name).ok()).max(Depth::Ansi16)
    }

    /// A depth `--color` names: `16`, `256`, `truecolor` or `24bit`.
    #[must_use]
    pub fn parse(s: &str) -> Option<Depth> {
        match s {
            "16" | "ansi" => Some(Depth::Ansi16),
            "256" | "ansi256" => Some(Depth::Ansi256),
            "truecolor" | "24bit" | "16m" => Some(Depth::TrueColor),
            _ => None,
        }
    }
}

/// Make the console ready for the codes the reports send: on Windows, its
/// escape-code processing switched on; true where the codes will be
/// rendered, false where the standard output is no console or the console
/// refuses. Every other system renders them as they are.
pub fn console_ready() -> bool {
    enable_vt()
}

/// Switch a Windows console's escape-code processing on, so the codes the
/// reports send are rendered rather than printed; true where they will be,
/// false where the standard output is no console or the console refuses.
#[cfg(windows)]
fn enable_vt() -> bool {
    match winapi_util::console::Console::stdout() {
        Ok(mut console) => match console.set_virtual_terminal_processing(true) {
            Ok(()) => true,
            Err(_refused) => false,
        },
        Err(_not_a_console) => false,
    }
}

#[cfg(not(windows))]
fn enable_vt() -> bool {
    true
}

/// The values xterm renders the sixteen base colors as, which is what a
/// 24-bit value is measured against when a console has only those.
const BASE_RGB: [(u8, u8, u8); 16] = [
    (0, 0, 0),
    (205, 0, 0),
    (0, 205, 0),
    (205, 205, 0),
    (0, 0, 238),
    (205, 0, 205),
    (0, 205, 205),
    (229, 229, 229),
    (127, 127, 127),
    (255, 0, 0),
    (0, 255, 0),
    (255, 255, 0),
    (92, 92, 255),
    (255, 0, 255),
    (0, 255, 255),
    (255, 255, 255),
];

/// The six levels each axis of the 256-color cube takes.
const CUBE: [u8; 6] = [0, 95, 135, 175, 215, 255];

/// One color, as a report holds it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Color {
    /// One of the console's sixteen: `0..8` the base eight, black to white,
    /// and `8..16` their bright forms.
    Base(u8),
    /// One entry of the 256-color table.
    Index(u8),
    /// A 24-bit value.
    Rgb(u8, u8, u8),
}

/// The base colors, in the order their codes count.
const NAMES: [&str; 8] = ["black", "red", "green", "yellow", "blue", "magenta", "cyan", "white"];

impl Color {
    /// A color as `--colors` writes one: a base color's name, `#rrggbb`,
    /// `0xRR,0xGG,0xBB`, or an entry of the 256-color table by number.
    ///
    /// # Errors
    ///
    /// Anything else, with what is accepted.
    pub fn parse(s: &str) -> Result<Color, String> {
        if let Some(i) = NAMES.iter().position(|n| *n == s) {
            return Ok(Color::Base(u8::try_from(i).unwrap_or(7)));
        }
        if let Some(hex) = s.strip_prefix('#') {
            if hex.len() != 6 {
                return Err(format!("{s:?} is not a #rrggbb color"));
            }
            let mut channels = [0u8; 3];
            for (slot, at) in channels.iter_mut().zip([0, 2, 4]) {
                *slot = u8::from_str_radix(&hex[at..at + 2], 16)
                    .map_err(|e| format!("{s:?} is not a #rrggbb color: {e}"))?;
            }
            return Ok(Color::Rgb(channels[0], channels[1], channels[2]));
        }
        if s.starts_with("0x") {
            let parts: Vec<&str> = s.split(',').collect();
            if parts.len() == 3 {
                let mut channels = [0u8; 3];
                for (slot, part) in channels.iter_mut().zip(&parts) {
                    let digits = part.strip_prefix("0x").ok_or_else(|| {
                        format!("{s:?} is not a 0xRR,0xGG,0xBB color")
                    })?;
                    *slot = u8::from_str_radix(digits, 16)
                        .map_err(|e| format!("{s:?} is not a 0xRR,0xGG,0xBB color: {e}"))?;
                }
                return Ok(Color::Rgb(channels[0], channels[1], channels[2]));
            }
            return Err(format!("{s:?} is not a 0xRR,0xGG,0xBB color"));
        }
        if s.bytes().all(|b| b.is_ascii_digit()) && !s.is_empty() {
            return match s.parse::<u8>() {
                Ok(i) => Ok(Color::Index(i)),
                Err(e) => Err(format!("{s:?} is not an entry of the 256-color table: {e}")),
            };
        }
        Err(format!(
            "{s:?} is not a color; write black, red, green, yellow, blue, magenta, cyan or white, #rrggbb, 0xRR,0xGG,0xBB, or a number 0-255"
        ))
    }

    /// The 24-bit value the color renders as, a base or table entry at
    /// xterm's value for it.
    #[must_use]
    pub fn rgb(self) -> (u8, u8, u8) {
        match self {
            Color::Rgb(r, g, b) => (r, g, b),
            Color::Base(i) => BASE_RGB[usize::from(i) % 16],
            Color::Index(i) if i < 16 => BASE_RGB[usize::from(i)],
            Color::Index(i) if i < 232 => {
                let i = usize::from(i) - 16;
                (CUBE[i / 36], CUBE[(i / 6) % 6], CUBE[i % 6])
            }
            Color::Index(i) => {
                let gray = 8 + 10 * (i - 232);
                (gray, gray, gray)
            }
        }
    }

    /// The color as `depth` renders it: the same where the depth has it, the
    /// nearest it has otherwise, and none at all where the depth is off.
    #[must_use]
    pub fn at(self, depth: Depth) -> Option<Color> {
        match (depth, self) {
            (Depth::Off, _) => None,
            (_, Color::Base(_)) | (Depth::TrueColor, _) | (Depth::Ansi256, Color::Index(_)) => Some(self),
            (Depth::Ansi256, Color::Rgb(..)) => Some(Color::Index(nearest_index(self.rgb()))),
            (Depth::Ansi16, _) => Some(Color::Base(nearest_base(self.rgb()))),
        }
    }

    /// The SGR parameters that select the color, in the foreground or the
    /// background.
    fn sgr(self, foreground: bool) -> String {
        match self {
            Color::Base(i) => {
                let (dim, bright) = if foreground { (30, 90) } else { (40, 100) };
                let i = u16::from(i % 16);
                if i < 8 { (dim + i).to_string() } else { (bright + i - 8).to_string() }
            }
            Color::Index(i) => format!("{};5;{i}", if foreground { 38 } else { 48 }),
            Color::Rgb(r, g, b) => format!("{};2;{r};{g};{b}", if foreground { 38 } else { 48 }),
        }
    }
}

/// The squared distance between two 24-bit values.
fn distance(a: (u8, u8, u8), b: (u8, u8, u8)) -> u32 {
    let d = |x: u8, y: u8| {
        let d = i32::from(x) - i32::from(y);
        (d * d).unsigned_abs()
    };
    d(a.0, b.0) + d(a.1, b.1) + d(a.2, b.2)
}

/// The entry of the 256-color cube and grays nearest a 24-bit value; the
/// first sixteen are left out, since their hue is the console's.
fn nearest_index(rgb: (u8, u8, u8)) -> u8 {
    (16..=255u8).min_by_key(|&i| distance(Color::Index(i).rgb(), rgb)).unwrap_or(16)
}

/// The base color nearest a 24-bit value, at xterm's values for the sixteen.
fn nearest_base(rgb: (u8, u8, u8)) -> u8 {
    (0..16u8).min_by_key(|&i| distance(BASE_RGB[usize::from(i)], rgb)).unwrap_or(0)
}

/// How one role is painted.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Style {
    /// The foreground color, if any.
    pub fg: Option<Color>,
    /// The background color, if any.
    pub bg: Option<Color>,
    /// Bold.
    pub bold: bool,
    /// A base foreground in its bright form.
    pub intense: bool,
    /// Underlined.
    pub underline: bool,
}

impl Style {
    /// The SGR parameters that open the style at `depth`, or nothing where
    /// the depth is off or the style paints nothing.
    #[must_use]
    pub fn sgr(&self, depth: Depth) -> String {
        if depth == Depth::Off {
            return String::new();
        }
        let mut parts: Vec<String> = Vec::new();
        if self.bold {
            parts.push("1".to_string());
        }
        if self.underline {
            parts.push("4".to_string());
        }
        if let Some(fg) = self.fg.and_then(|c| c.at(depth)) {
            let fg = match fg {
                Color::Base(i) if self.intense && i < 8 => Color::Base(i + 8),
                other => other,
            };
            parts.push(fg.sgr(true));
        }
        if let Some(bg) = self.bg.and_then(|c| c.at(depth)) {
            parts.push(bg.sgr(false));
        }
        parts.join(";")
    }
}

/// How much of a printed line the token kinds paint.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum KindPaint {
    /// No kind is painted; only the roles are.
    None,
    /// The recognized value kinds and quoted strings. Words, punctuation,
    /// brackets and whitespace are left plain, which is what keeps the
    /// painted tokens legible: a line where every token carries a color is a
    /// line where the color says nothing.
    #[default]
    Values,
    /// Words, punctuation and brackets too, so a line reads as an editor
    /// paints source.
    All,
}

impl KindPaint {
    /// The level a `kind:*` spec or the environment names.
    #[must_use]
    pub fn parse(s: &str) -> Option<KindPaint> {
        match s {
            "none" | "off" => Some(KindPaint::None),
            "values" => Some(KindPaint::Values),
            "all" => Some(KindPaint::All),
            _ => None,
        }
    }

    /// Whether a kind is painted at this level.
    #[must_use]
    pub fn paints(self, kind: TokenKind) -> bool {
        match self {
            KindPaint::None => false,
            KindPaint::All => true,
            KindPaint::Values => !matches!(
                kind,
                TokenKind::Word
                    | TokenKind::Punct
                    | TokenKind::Open(_)
                    | TokenKind::Close(_)
                    | TokenKind::Whitespace
                    | TokenKind::Other
            ),
        }
    }
}

/// The color each token kind is painted, as one 24-bit value per kind.
///
/// Written once, at full depth, because [`Color::at`] already carries a value
/// down the ladder: to the nearest entry of the 256-color cube on a
/// 256-color console and to the nearest of the sixteen on a 16-color one. So
/// the family structure is never written twice. Kinds that answer the same
/// question are in one arc of the hue wheel, which means they stay apart
/// where the console can tell them apart and collapse onto their family's
/// base color where it cannot.
///
/// Every value is an entry of the 256-color cube written out as its 24-bit
/// form, which is what makes the ladder lossless down to 256: a color
/// already on a cube entry snaps to itself, so a 256-color console
/// renders exactly what a 24-bit one does and no two kinds can collide on the
/// way down. The cube holds 216 colors, which is room enough for two dozen
/// well separated hues, and the only lossy step left is the one to sixteen -
/// which is the step where the families are supposed to take over.
///
/// Every value is saturated on purpose. The sixteen a console renders are
/// fully saturated primaries plus black, gray and white, so a pale or muted
/// value snaps to gray rather than to a hue, and a family would lose its
/// color exactly where it most needs one. Green stays out of the blue arc and
/// red stays out of the magenta arc for the same reason: the nearest of the
/// sixteen is measured in unweighted rgb, where a little green pulls a blue
/// into cyan and a little blue pulls a red into magenta.
///
/// The arcs, and the question each answers:
///
/// - blue through cyan, where: an address, a route, a place to reach.
/// - amber through gold, how much: a quantity, in whatever unit.
/// - green, when: an instant.
/// - violet through magenta, which: an opaque identifier, standing for a
///   thing rather than describing it.
/// - red, alarm: a value that is a finding when it appears in output. These
///   also carry an underline, which is an attribute rather than a color and
///   so survives every depth and every kind of color blindness.
const KIND_COLORS: [(TokenKind, Color); 25] = [
    // Where: blue, kept clear of green so it does not read as cyan.
    (TokenKind::Ip, Color::Rgb(0x00, 0x5f, 0xff)),
    (TokenKind::Cidr, Color::Rgb(0x00, 0x00, 0xd7)),
    (TokenKind::Mac, Color::Rgb(0x5f, 0x87, 0xff)),
    (TokenKind::Url, Color::Rgb(0x00, 0x5f, 0xd7)),
    (TokenKind::Email, Color::Rgb(0x5f, 0x5f, 0xff)),
    (TokenKind::Phone, Color::Rgb(0x00, 0x00, 0xaf)),
    // Where, at the cyan end: a place on a disk or on the earth.
    (TokenKind::Path, Color::Rgb(0x00, 0xd7, 0xd7)),
    (TokenKind::Geo, Color::Rgb(0x00, 0xaf, 0xaf)),
    // How much.
    (TokenKind::Number, Color::Rgb(0xff, 0xd7, 0x00)),
    (TokenKind::Percent, Color::Rgb(0xff, 0xaf, 0x00)),
    (TokenKind::ByteSize, Color::Rgb(0xd7, 0xaf, 0x00)),
    (TokenKind::Money, Color::Rgb(0xff, 0xff, 0x5f)),
    (TokenKind::Duration, Color::Rgb(0xd7, 0x87, 0x00)),
    (TokenKind::Quantity, Color::Rgb(0xaf, 0x87, 0x00)),
    // When.
    (TokenKind::Timestamp, Color::Rgb(0x00, 0xd7, 0x5f)),
    // Which: magenta, kept red-heavy so it does not read as blue.
    (TokenKind::Uuid, Color::Rgb(0xd7, 0x5f, 0xd7)),
    (TokenKind::Version, Color::Rgb(0xff, 0x5f, 0xff)),
    (TokenKind::HexColor, Color::Rgb(0xaf, 0x00, 0xaf)),
    (TokenKind::Quoted, Color::Rgb(0xd7, 0x00, 0xd7)),
    // Alarm, kept clear of blue so it does not read as magenta and clear of
    // green so it does not read as yellow.
    (TokenKind::CreditCard, Color::Rgb(0xff, 0x00, 0x00)),
    (TokenKind::Jwt, Color::Rgb(0xd7, 0x00, 0x00)),
    (TokenKind::Base64, Color::Rgb(0xff, 0x00, 0x5f)),
    (TokenKind::HashDigest, Color::Rgb(0x87, 0x00, 0x00)),
    (TokenKind::Hex, Color::Rgb(0xaf, 0x00, 0x00)),
    // Painted only where the level is `all` and a line reads as source, so it
    // is the one entry meant to recede rather than stand out.
    (TokenKind::Word, Color::Rgb(0xaf, 0xaf, 0xaf)),
];

/// The kinds that are a finding when they appear in output, and so carry an
/// underline beside their hue.
const ALARM_KINDS: [TokenKind; 5] =
    [TokenKind::CreditCard, TokenKind::Jwt, TokenKind::Base64, TokenKind::HashDigest, TokenKind::Hex];

/// How `kind` is painted before any `--colors` override.
#[must_use]
pub fn kind_style(kind: TokenKind) -> Style {
    let fg = KIND_COLORS.iter().find(|(k, _)| *k == kind).map(|&(_, c)| c);
    Style { fg, underline: ALARM_KINDS.contains(&kind), ..Style::default() }
}

/// What a report paints.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Role {
    /// The matched text.
    Match,
    /// The input's path.
    Path,
    /// A line number.
    Line,
    /// A column number.
    Column,
    /// The `:` and `-` between them.
    Separator,
}

impl Role {
    /// Every role, in the order a palette holds them.
    pub const ALL: [Role; 5] = [Role::Match, Role::Path, Role::Line, Role::Column, Role::Separator];

    /// A role as `--colors` names it.
    #[must_use]
    pub fn parse(s: &str) -> Option<Role> {
        match s {
            "match" => Some(Role::Match),
            "path" => Some(Role::Path),
            "line" => Some(Role::Line),
            "column" => Some(Role::Column),
            "separator" => Some(Role::Separator),
            _ => None,
        }
    }

    fn slot(self) -> usize {
        match self {
            Role::Match => 0,
            Role::Path => 1,
            Role::Line => 2,
            Role::Column => 3,
            Role::Separator => 4,
        }
    }
}

/// The style of every role, with the kind and register overrides a report
/// paints the printed lines by.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Palette {
    styles: [Style; 5],
    /// How much of a line the kinds paint.
    kinds: KindPaint,
    /// Kinds written with a `kind:NAME` spec, over the built-in scheme. A
    /// name given twice keeps the last, which is how a later `--colors`
    /// wins.
    kind_overrides: Vec<(TokenKind, Style)>,
    /// Registers written with a `capture:NAME` spec. A register painted this
    /// way paints over the kind of the tokens it binds, because it is the
    /// more specific thing the user asked for.
    capture_overrides: Vec<(String, Style)>,
}

impl Default for Palette {
    fn default() -> Self {
        Palette::base()
    }
}

impl Palette {
    /// The console's own base colors, as grep paints its output: the match
    /// bold red, the path magenta, the line and the column green, the
    /// separators cyan. Sent as the base codes at every depth, so the
    /// console's theme decides the hue.
    #[must_use]
    pub fn base() -> Palette {
        let fg = |i: u8| Style { fg: Some(Color::Base(i)), ..Style::default() };
        Palette {
            styles: [
                Style { fg: Some(Color::Base(1)), bold: true, ..Style::default() },
                fg(5),
                fg(2),
                fg(2),
                fg(6),
            ],
            kinds: KindPaint::default(),
            kind_overrides: Vec::new(),
            capture_overrides: Vec::new(),
        }
    }

    /// A palette painting nothing.
    #[must_use]
    pub fn plain() -> Palette {
        Palette {
            styles: [Style::default(); 5],
            kinds: KindPaint::None,
            kind_overrides: Vec::new(),
            capture_overrides: Vec::new(),
        }
    }

    /// How `role` is painted.
    #[must_use]
    pub fn style(&self, role: Role) -> Style {
        self.styles[role.slot()]
    }

    /// How much of a line the kinds paint.
    #[must_use]
    pub fn kind_paint(&self) -> KindPaint {
        self.kinds
    }

    /// Set how much of a line the kinds paint, as the environment asks.
    pub fn set_kind_paint(&mut self, level: KindPaint) {
        self.kinds = level;
    }

    /// How a token of `kind` is painted, or `None` where the level leaves it
    /// plain and no spec named it.
    ///
    /// A spec naming the kind wins over the built-in scheme, and it wins over
    /// the level too: asking for `kind:word:fg:blue` is asking for words to
    /// be painted, whatever the level would have done with them.
    #[must_use]
    pub fn kind(&self, kind: TokenKind) -> Option<Style> {
        if let Some((_, style)) = self.kind_overrides.iter().rev().find(|(k, _)| *k == kind) {
            return Some(*style);
        }
        if !self.kinds.paints(kind) {
            return None;
        }
        let style = kind_style(kind);
        (style != Style::default()).then_some(style)
    }

    /// How the tokens a register binds are painted, where a spec named it.
    #[must_use]
    pub fn capture(&self, name: &str) -> Option<Style> {
        self.capture_overrides.iter().rev().find(|(n, _)| n == name).map(|(_, style)| *style)
    }

    /// Whether any register is painted, so a report knows to resolve them.
    #[must_use]
    pub fn paints_captures(&self) -> bool {
        !self.capture_overrides.is_empty()
    }

    /// Apply one `--colors` spec, as ripgrep spells it: `ROLE:fg:COLOR`,
    /// `ROLE:bg:COLOR`, `ROLE:style:bold|nobold|intense|nointense|underline|nounderline`,
    /// or `ROLE:none` to paint the role plainly, with the role one of
    /// `match`, `path`, `line`, `column` and `separator`.
    ///
    /// # Errors
    ///
    /// A spec that is not one of those, naming what is expected.
    pub fn set(&mut self, spec: &str) -> Result<(), String> {
        // `kind:NAME` and `capture:NAME` carry a name before the rest, so
        // they are split one field deeper than a role's spec.
        if let Some(rest) = spec.strip_prefix("kind:") {
            return self.set_kind(spec, rest);
        }
        if let Some(rest) = spec.strip_prefix("capture:") {
            let (name, rest) = rest.split_once(':').ok_or_else(|| {
                format!("{spec:?} names a register and nothing to paint it; write capture:NAME:fg:COLOR")
            })?;
            if name.is_empty() {
                return Err(format!("{spec:?} names no register; write capture:NAME:fg:COLOR"));
            }
            // A second spec for one register builds on the first, so
            // `capture:host:fg:blue` then `capture:host:style:bold` is one
            // bold blue register rather than a bold plain one.
            #[expect(
                clippy::manual_unwrap_or_default,
                reason = "an Option default, which the repository's swallowed-error guard reads as a Result default"
            )]
            let mut style = match self.capture(name) {
                Some(already) => already,
                None => Style::default(),
            };
            apply(spec, rest, &mut style)?;
            self.capture_overrides.push((name.to_string(), style));
            return Ok(());
        }
        let mut parts = spec.splitn(3, ':');
        let role = parts.next().unwrap_or("");
        let role = Role::parse(role).ok_or_else(|| {
            format!(
                "{spec:?} names no role; a spec opens with match, path, line, column, separator, kind:NAME or capture:NAME"
            )
        })?;
        let rest = match (parts.next(), parts.next()) {
            (Some(head), Some(tail)) => format!("{head}:{tail}"),
            (Some(head), None) => head.to_string(),
            (None, _) => String::new(),
        };
        apply(spec, &rest, &mut self.styles[role.slot()])
    }

    /// One `kind:...` spec: `kind:*:LEVEL` sets how much of a line the kinds
    /// paint, and `kind:NAME:...` paints one kind over the built-in scheme.
    fn set_kind(&mut self, spec: &str, rest: &str) -> Result<(), String> {
        let (name, rest) = rest.split_once(':').ok_or_else(|| {
            format!("{spec:?} names a kind and nothing to paint it; write kind:NAME:fg:COLOR or kind:*:LEVEL")
        })?;
        if name == "*" {
            // `kind:*` speaks for every kind at once, and what it takes is a
            // level rather than a color: painting two dozen kinds one color
            // would be the same as painting none.
            let level = KindPaint::parse(rest).ok_or_else(|| {
                format!("{rest:?} is not a level; kind:* takes none, values or all")
            })?;
            self.kinds = level;
            self.kind_overrides.clear();
            return Ok(());
        }
        let kind = TokenKind::named(name).ok_or_else(|| {
            format!("{name:?} is no token kind; kind:* speaks for all of them")
        })?;
        #[expect(
            clippy::manual_unwrap_or_default,
            reason = "an Option default, which the repository's swallowed-error guard reads as a Result default"
        )]
        let mut style = match self.kind(kind) {
            Some(already) => already,
            None => Style::default(),
        };
        apply(spec, rest, &mut style)?;
        self.kind_overrides.push((kind, style));
        Ok(())
    }
}

/// Apply the `fg:COLOR`, `bg:COLOR`, `style:STYLE` or `none` half of a spec
/// to one style, `spec` being the whole of what the user wrote, for the
/// message where it is not one of those.
fn apply(spec: &str, rest: &str, style: &mut Style) -> Result<(), String> {
    let (head, tail) = match rest.split_once(':') {
        Some((head, tail)) => (head, Some(tail)),
        None => (rest, None),
    };
    match (head, tail) {
        ("none", None) => *style = Style::default(),
        ("fg", Some(color)) => style.fg = Some(Color::parse(color)?),
        ("bg", Some(color)) => style.bg = Some(Color::parse(color)?),
        ("style", Some(which)) => match which {
            "bold" => style.bold = true,
            "nobold" => style.bold = false,
            "intense" => style.intense = true,
            "nointense" => style.intense = false,
            "underline" => style.underline = true,
            "nounderline" => style.underline = false,
            other => {
                return Err(format!(
                    "{other:?} is not a style; write bold, nobold, intense, nointense, underline or nounderline"
                ));
            }
        },
        _ => {
            return Err(format!(
                "{spec:?} is not a color spec; write NAME:fg:COLOR, NAME:bg:COLOR, NAME:style:STYLE or NAME:none"
            ));
        }
    }
    Ok(())
}

/// A palette at a depth: what a report paints with.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Painter {
    depth: Depth,
    palette: Palette,
}

impl Painter {
    /// `palette` sent at `depth`.
    #[must_use]
    pub fn new(depth: Depth, palette: Palette) -> Painter {
        Painter { depth, palette }
    }

    /// A painter that paints nothing.
    #[must_use]
    pub fn off() -> Painter {
        Painter { depth: Depth::Off, palette: Palette::plain() }
    }

    /// The depth the painter sends at.
    #[must_use]
    pub fn depth(&self) -> Depth {
        self.depth
    }

    /// Whether anything is painted at all.
    #[must_use]
    pub fn is_on(&self) -> bool {
        self.depth != Depth::Off
    }

    /// The palette the painter paints from.
    #[must_use]
    pub fn palette(&self) -> &Palette {
        &self.palette
    }

    /// Whether any kind is painted, so a report knows whether it needs the
    /// tokens at all.
    #[must_use]
    pub fn paints_kinds(&self) -> bool {
        self.is_on()
            && (self.palette.kind_paint() != KindPaint::None
                || !self.palette.kind_overrides.is_empty()
                || self.palette.paints_captures())
    }

    /// The bytes of `input` over `span`, with every token a kind paints
    /// wrapped in that kind's style.
    ///
    /// `tokens` are the whole input's, in order; the ones overlapping `span`
    /// are found by search rather than by scanning the list, so painting one
    /// line of a long input costs what the line costs.
    ///
    /// A token reaching past either end of `span` is painted over the part
    /// inside it and left alone outside, so a line of a token that spans
    /// lines is painted as the line it is.
    #[must_use]
    pub fn paint_kinds(&self, input: &[u8], span: std::ops::Range<usize>, tokens: &[Token]) -> String {
        let plain = || String::from_utf8_lossy(&input[span.clone()]).into_owned();
        if !self.paints_kinds() {
            return plain();
        }
        let first = tokens.partition_point(|t| t.end() <= span.start);
        let mut out = String::with_capacity(span.len());
        let mut at = span.start;
        for token in &tokens[first..] {
            if token.start() >= span.end {
                break;
            }
            let Some(style) = self.palette.kind(token.kind) else { continue };
            let sgr = style.sgr(self.depth);
            if sgr.is_empty() {
                continue;
            }
            let from = token.start().max(span.start);
            let to = token.end().min(span.end);
            if from < at {
                continue;
            }
            out.push_str(&String::from_utf8_lossy(&input[at..from]));
            out.push_str("\x1b[");
            out.push_str(&sgr);
            out.push('m');
            out.push_str(&String::from_utf8_lossy(&input[from..to]));
            out.push_str("\x1b[0m");
            at = to;
        }
        out.push_str(&String::from_utf8_lossy(&input[at..span.end]));
        out
    }

    /// `text` in the role's style, or as it is where nothing paints it.
    #[must_use]
    pub fn paint(&self, role: Role, text: &str) -> String {
        let sgr = self.palette.style(role).sgr(self.depth);
        if sgr.is_empty() {
            return text.to_string();
        }
        format!("\x1b[{sgr}m{text}\x1b[0m")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env_of(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> {
        let pairs: Vec<(String, String)> =
            pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
        move |name: &str| pairs.iter().find(|(k, _)| k == name).map(|(_, v)| v.clone())
    }

    /// The base color a kind's 24-bit value lands on where the console has
    /// only sixteen, by name. The bright eight carry the same names as the
    /// dim eight, since a family is the hue and not the brightness - but
    /// `black` names index 0 and `gray` its bright form, because a color
    /// landing on gray has lost its hue rather than kept it, which is the
    /// failure this is here to catch.
    fn collapses_to(kind: TokenKind) -> &'static str {
        let fg = kind_style(kind).fg.expect("every listed kind carries a color");
        match fg.at(Depth::Ansi16).expect("sixteen colors is not off") {
            Color::Base(8) => "gray",
            Color::Base(i) => NAMES[usize::from(i) % 8],
            other => panic!("sixteen colors gave {other:?}, which is not one of them"),
        }
    }

    #[test]
    fn every_kind_collapses_onto_its_own_family_at_sixteen_colors() {
        // The whole scheme rests on this. Each color is written once at full
        // depth and carried down by `Color::at`, so a family holds together
        // on a poor console only if its arc lands on one base color. A value
        // too pale or too close to a neighboring arc lands somewhere else,
        // and no amount of reading the hex says which - this measures it.
        let families: [(&str, &[TokenKind]); 5] = [
            ("blue", &[TokenKind::Ip, TokenKind::Cidr, TokenKind::Mac, TokenKind::Url, TokenKind::Email, TokenKind::Phone]),
            ("cyan", &[TokenKind::Path, TokenKind::Geo]),
            ("yellow", &[TokenKind::Number, TokenKind::Percent, TokenKind::ByteSize, TokenKind::Money, TokenKind::Duration, TokenKind::Quantity]),
            ("green", &[TokenKind::Timestamp]),
            ("magenta", &[TokenKind::Uuid, TokenKind::Version, TokenKind::HexColor, TokenKind::Quoted]),
        ];
        for (base, kinds) in families {
            for kind in kinds {
                assert_eq!(collapses_to(*kind), base, "{} left its family", kind.name());
            }
        }
        for kind in ALARM_KINDS {
            assert_eq!(collapses_to(kind), "red", "{} left the alarm family", kind.name());
        }
    }

    #[test]
    fn the_kinds_stay_apart_where_the_console_can_tell_them_apart() {
        // The other half of the claim: the arcs collapse at sixteen, and do
        // NOT collapse above it. A scheme whose kinds all snapped to one
        // entry at 256 would pass the test above and still show six colors
        // on a console that can render two hundred.
        let mut at_256: Vec<Color> = KIND_COLORS
            .iter()
            .map(|(_, c)| c.at(Depth::Ansi256).expect("256 colors is not off"))
            .collect();
        at_256.sort_by_key(|c| c.rgb());
        at_256.dedup();
        assert_eq!(at_256.len(), KIND_COLORS.len(), "two kinds share an entry at 256 colors");
        // Every value is on a cube entry already, so the step from 24 bits
        // to 256 moves nothing: the two depths render the same colors, and
        // the only lossy step is the one to sixteen.
        for (kind, color) in KIND_COLORS {
            assert_eq!(color.at(Depth::TrueColor), Some(color), "{} moved at 24 bits", kind.name());
            let at_256 = color.at(Depth::Ansi256).expect("256 colors is not off");
            assert_eq!(at_256.rgb(), color.rgb(), "{} is off the 256-color cube", kind.name());
        }
    }

    #[test]
    fn the_alarm_kinds_carry_an_underline_at_every_depth() {
        // The hue merges into the family at sixteen colors, so the underline
        // is what still says "this is a finding" there, and to a reader who
        // cannot tell the hue from its neighbors at any depth.
        for kind in ALARM_KINDS {
            let style = kind_style(kind);
            assert!(style.underline, "{} carries no underline", kind.name());
            for depth in [Depth::Ansi16, Depth::Ansi256, Depth::TrueColor] {
                assert!(style.sgr(depth).split(';').any(|p| p == "4"), "{} at {depth:?}", kind.name());
            }
        }
        // A kind that is not a finding carries none, or the attribute would
        // stop meaning anything.
        assert!(!kind_style(TokenKind::Number).underline);
    }

    #[test]
    fn the_levels_decide_which_kinds_are_painted() {
        assert!(!KindPaint::None.paints(TokenKind::Ip));
        assert!(KindPaint::Values.paints(TokenKind::Ip));
        assert!(KindPaint::Values.paints(TokenKind::Quoted), "a quoted string is a value");
        assert!(!KindPaint::Values.paints(TokenKind::Word), "words are the quiet part of a line");
        assert!(!KindPaint::Values.paints(TokenKind::Punct));
        assert!(KindPaint::All.paints(TokenKind::Word));
        assert!(KindPaint::All.paints(TokenKind::Punct));
        assert_eq!(KindPaint::parse("none"), Some(KindPaint::None));
        assert_eq!(KindPaint::parse("values"), Some(KindPaint::Values));
        assert_eq!(KindPaint::parse("all"), Some(KindPaint::All));
        assert_eq!(KindPaint::parse("sometimes"), None);
        assert_eq!(KindPaint::default(), KindPaint::Values);
    }

    #[test]
    fn the_depth_follows_the_environment_and_the_terminal() {
        assert_eq!(Depth::of(false, &env_of(&[("COLORTERM", "truecolor")])), Depth::Off);
        assert_eq!(Depth::of(true, &env_of(&[("NO_COLOR", "1"), ("COLORTERM", "truecolor")])), Depth::Off);
        assert_eq!(Depth::of(true, &env_of(&[("TERM", "dumb")])), Depth::Off);
        assert_eq!(Depth::of(true, &env_of(&[("TERM", "xterm"), ("COLORTERM", "24bit")])), Depth::TrueColor);
        assert_eq!(Depth::of(true, &env_of(&[("TERM", "xterm-256color"), ("WT_SESSION", "x")])), Depth::TrueColor);
        assert_eq!(Depth::of(true, &env_of(&[("TERM", "xterm-256color")])), Depth::Ansi256);
        assert_eq!(Depth::of(true, &env_of(&[("TERM", "xterm-kitty")])), Depth::TrueColor);
        assert_eq!(Depth::of(true, &env_of(&[("TERM", "xterm")])), Depth::Ansi16);
        assert_eq!(Depth::parse("256"), Some(Depth::Ansi256));
        assert_eq!(Depth::parse("24bit"), Some(Depth::TrueColor));
        assert_eq!(Depth::parse("lots"), None);
    }

    #[test]
    fn a_color_is_sent_at_the_depth_or_at_the_nearest_the_depth_has() {
        assert_eq!(Color::parse("red").expect("a name"), Color::Base(1));
        assert_eq!(Color::parse("#ff6b6b").expect("hex"), Color::Rgb(0xff, 0x6b, 0x6b));
        assert_eq!(Color::parse("0x00,0xff,0x10").expect("rg's form"), Color::Rgb(0, 255, 16));
        assert_eq!(Color::parse("208").expect("a table entry"), Color::Index(208));
        assert!(Color::parse("#12").expect_err("short hex").contains("#rrggbb"));
        assert!(Color::parse("pinkish").expect_err("a word").contains("is not a color"));
        assert_eq!(Color::Index(208).rgb(), (255, 135, 0));
        assert_eq!(Color::Index(244).rgb(), (128, 128, 128));
        assert_eq!(Color::Rgb(255, 0, 0).at(Depth::Ansi16), Some(Color::Base(9)));
        assert_eq!(Color::Rgb(200, 0, 0).at(Depth::Ansi16), Some(Color::Base(1)));
        assert_eq!(Color::Rgb(255, 135, 0).at(Depth::Ansi256), Some(Color::Index(208)));
        assert_eq!(Color::Rgb(255, 135, 0).at(Depth::TrueColor), Some(Color::Rgb(255, 135, 0)));
        assert_eq!(Color::Base(5).at(Depth::TrueColor), Some(Color::Base(5)));
        // Orange is nearer xterm's yellow (205,205,0) than its bright yellow.
        assert_eq!(Color::Index(208).at(Depth::Ansi16), Some(Color::Base(3)));
        assert_eq!(Color::Base(5).at(Depth::Off), None);
    }

    #[test]
    fn the_palette_takes_overrides_as_ripgrep_spells_them() {
        let mut p = Palette::base();
        assert_eq!(p.style(Role::Match), Style { fg: Some(Color::Base(1)), bold: true, ..Style::default() });
        p.set("match:fg:#ff6b6b").expect("fg");
        p.set("match:style:nobold").expect("style");
        p.set("path:bg:blue").expect("bg");
        p.set("line:style:underline").expect("underline");
        p.set("separator:none").expect("none");
        assert_eq!(p.style(Role::Match), Style { fg: Some(Color::Rgb(0xff, 0x6b, 0x6b)), ..Style::default() });
        assert_eq!(p.style(Role::Path).bg, Some(Color::Base(4)));
        assert!(p.style(Role::Line).underline);
        assert_eq!(p.style(Role::Separator), Style::default());
        assert!(p.set("tone:fg:red").expect_err("no role").contains("names no role"));
        assert!(p.set("match:weight:bold").expect_err("no attribute").contains("is not a color spec"));
        assert!(p.set("match:style:loud").expect_err("no style").contains("is not a style"));
    }

    #[test]
    fn a_painter_wraps_text_in_the_codes_its_depth_renders() {
        let base = Painter::new(Depth::Ansi16, Palette::base());
        assert_eq!(base.paint(Role::Match, "x"), "\x1b[1;31mx\x1b[0m");
        assert_eq!(base.paint(Role::Path, "p"), "\x1b[35mp\x1b[0m");
        assert_eq!(base.paint(Role::Separator, ":"), "\x1b[36m:\x1b[0m");
        let mut palette = Palette::base();
        palette.set("match:fg:#ff8700").expect("fg");
        palette.set("match:style:intense").expect("intense");
        palette.set("path:style:intense").expect("intense");
        assert_eq!(Painter::new(Depth::TrueColor, palette.clone()).paint(Role::Match, "x"), "\x1b[1;38;2;255;135;0mx\x1b[0m");
        assert_eq!(Painter::new(Depth::Ansi256, palette.clone()).paint(Role::Match, "x"), "\x1b[1;38;5;208mx\x1b[0m");
        assert_eq!(Painter::new(Depth::Ansi16, palette.clone()).paint(Role::Match, "x"), "\x1b[1;93mx\x1b[0m");
        assert_eq!(Painter::new(Depth::Ansi16, palette).paint(Role::Path, "p"), "\x1b[95mp\x1b[0m");
        assert_eq!(Painter::off().paint(Role::Match, "x"), "x");
        assert!(!Painter::off().is_on());
    }
}
