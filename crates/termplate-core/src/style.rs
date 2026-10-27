//! Appearance (RLML D 9.1, D 9.3; spec §11).
//!
//! ```text
//! colors
//! - STONE 5a5a6e                     palette: NAME and six lower-case hex digits
//!
//! panels
//! - LOG border single fg STONE bg black
//! - MENU.selected fg black bg cyan bold
//! - MENU.disabled fg dark_grey
//!
//! logs
//! - LOG.stderr fg red
//! ```
//!
//! Colors are palette names, hex values or the terminal's own colors
//! (`red`, `dark_grey`, `default`: lower-case vocabulary). A row applies to
//! a layout area, a menu, a log, a panel or a key context (`MENU`); the
//! `.variant` part styles one role inside it. `NO_COLOR` drops colors and
//! keeps attributes.

use std::collections::BTreeMap;

use crate::frame::Role;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Color {
    Reset,
    Black,
    Red,
    Green,
    Yellow,
    Blue,
    Magenta,
    Cyan,
    White,
    Grey,
    DarkRed,
    DarkGreen,
    DarkYellow,
    DarkBlue,
    DarkMagenta,
    DarkCyan,
    DarkGrey,
    Rgb(u8, u8, u8),
}

pub const TERMINAL_COLORS: &[(&str, Color)] = &[
    ("default", Color::Reset),
    ("black", Color::Black),
    ("red", Color::Red),
    ("green", Color::Green),
    ("yellow", Color::Yellow),
    ("blue", Color::Blue),
    ("magenta", Color::Magenta),
    ("cyan", Color::Cyan),
    ("white", Color::White),
    ("grey", Color::Grey),
    ("dark_red", Color::DarkRed),
    ("dark_green", Color::DarkGreen),
    ("dark_yellow", Color::DarkYellow),
    ("dark_blue", Color::DarkBlue),
    ("dark_magenta", Color::DarkMagenta),
    ("dark_cyan", Color::DarkCyan),
    ("dark_grey", Color::DarkGrey),
];

/// `5a5a6e` (D 9.1: six lower-case hex digits, no `#`).
pub fn parse_hex(s: &str) -> Option<Color> {
    if s.len() != 6 || !s.chars().all(|c| c.is_ascii_digit() || ('a'..='f').contains(&c)) {
        return None;
    }
    let n = u32::from_str_radix(s, 16).ok()?;
    Some(Color::Rgb((n >> 16) as u8, (n >> 8) as u8, n as u8))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Hash)]
pub struct Attrs {
    pub bold: bool,
    pub dim: bool,
    pub italic: bool,
    pub underline: bool,
    pub reverse: bool,
}

pub const ATTR_NAMES: &[&str] = &["bold", "dim", "italic", "underline", "reverse"];

impl Attrs {
    fn set(&mut self, name: &str) -> bool {
        match name {
            "bold" => self.bold = true,
            "dim" => self.dim = true,
            "italic" => self.italic = true,
            "underline" => self.underline = true,
            "reverse" => self.reverse = true,
            _ => return false,
        }
        true
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Hash)]
pub struct Style {
    pub fg: Option<Color>,
    pub bg: Option<Color>,
    pub attrs: Attrs,
}

impl Style {
    /// `other` on top of `self`: set colors replace, attributes add up.
    pub fn over(self, other: Style) -> Style {
        Style {
            fg: other.fg.or(self.fg),
            bg: other.bg.or(self.bg),
            attrs: Attrs {
                bold: self.attrs.bold || other.attrs.bold,
                dim: self.attrs.dim || other.attrs.dim,
                italic: self.attrs.italic || other.attrs.italic,
                underline: self.attrs.underline || other.attrs.underline,
                reverse: self.attrs.reverse || other.attrs.reverse,
            },
        }
    }

    pub fn without_colors(self) -> Style {
        Style { fg: None, bg: None, attrs: self.attrs }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Border {
    #[default]
    None,
    Single,
    Double,
    Rounded,
    Heavy,
    Ascii,
}

impl Border {
    pub fn parse(s: &str) -> Option<Border> {
        Some(match s {
            "none" => Border::None,
            "single" => Border::Single,
            "double" => Border::Double,
            "rounded" => Border::Rounded,
            "heavy" => Border::Heavy,
            "ascii" => Border::Ascii,
            _ => return None,
        })
    }

    /// (horizontal, vertical, top-left, top-right, bottom-left, bottom-right)
    pub fn chars(self) -> Option<[char; 6]> {
        Some(match self {
            Border::None => return None,
            Border::Single => ['─', '│', '┌', '┐', '└', '┘'],
            Border::Double => ['═', '║', '╔', '╗', '╚', '╝'],
            Border::Rounded => ['─', '│', '╭', '╮', '╰', '╯'],
            Border::Heavy => ['━', '┃', '┏', '┓', '┗', '┛'],
            Border::Ascii => ['-', '|', '+', '+', '+', '+'],
        })
    }
}

/// Variant names a style row may use (`MENU.selected`).
pub const VARIANTS: &[&str] = &[
    "selected", "disabled", "title", "header", "input", "field", "bar", "border", "stderr", "status", "warning", "dim",
];

#[derive(Debug, Clone, PartialEq, Default)]
pub struct StyleRow {
    pub style: Style,
    pub border: Option<Border>,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct StyleSheet {
    pub palette: BTreeMap<String, Color>,
    /// `NAME` or `NAME.variant` → row.
    pub rows: BTreeMap<String, StyleRow>,
}

impl StyleSheet {
    pub fn color(&self, s: &str) -> Option<Color> {
        if let Some(c) = self.palette.get(s) {
            return Some(*c);
        }
        if let Some((_, c)) = TERMINAL_COLORS.iter().find(|(n, _)| *n == s) {
            return Some(*c);
        }
        parse_hex(s)
    }

    /// Parses the words of a style row after its name.
    pub fn parse_row(&self, words: &[&str]) -> Result<StyleRow, String> {
        let mut row = StyleRow::default();
        let mut i = 0;
        while i < words.len() {
            match words[i] {
                w @ ("fg" | "bg") => {
                    let v = words.get(i + 1).ok_or_else(|| format!("`{w}` needs a color"))?;
                    let c = self.color(v).ok_or_else(|| format!("unknown color `{v}`"))?;
                    if w == "fg" {
                        row.style.fg = Some(c);
                    } else {
                        row.style.bg = Some(c);
                    }
                    i += 2;
                }
                "border" => {
                    let v = words.get(i + 1).ok_or("`border` needs a style (single, double, rounded, heavy, ascii, none)")?;
                    row.border = Some(Border::parse(v).ok_or_else(|| format!("unknown border `{v}`"))?);
                    i += 2;
                }
                a if row.style.attrs.set(a) => i += 1,
                other => return Err(format!("unknown style word `{other}` (fg, bg, border, {})", ATTR_NAMES.join(", "))),
            }
        }
        Ok(row)
    }

    /// Style for `role` inside the areas `chain` (innermost last), on top
    /// of the built-in look.
    pub fn resolve(&self, chain: &[String], role: Role) -> Style {
        let mut style = builtin(role);
        for name in chain {
            if let Some(r) = self.rows.get(name) {
                style = style.over(Style { attrs: Attrs::default(), ..r.style });
            }
        }
        for name in chain {
            if let Some(r) = self.rows.get(&format!("{name}.{}", role.name())) {
                style = style.over(r.style);
            }
        }
        style
    }

    /// The border configured for the innermost name that has one.
    pub fn border(&self, chain: &[String]) -> Option<Border> {
        chain.iter().rev().find_map(|n| self.rows.get(n).and_then(|r| r.border))
    }
}

/// The built-in look of each role; selection uses reverse video so that it
/// shows without color.
pub fn builtin(role: Role) -> Style {
    let a = |f: fn(&mut Attrs)| {
        let mut x = Attrs::default();
        f(&mut x);
        x
    };
    match role {
        Role::Title => Style { attrs: a(|x| x.bold = true), ..Default::default() },
        Role::Field => Style { fg: Some(Color::Cyan), ..Default::default() },
        Role::Selected => Style { attrs: a(|x| x.reverse = true), ..Default::default() },
        Role::Disabled => Style { fg: Some(Color::DarkGrey), ..Default::default() },
        Role::Dim | Role::Input => Style { attrs: a(|x| x.underline = true), ..Default::default() },
        Role::Stderr => Style { fg: Some(Color::Red), ..Default::default() },
        Role::Warning => Style { fg: Some(Color::Yellow), attrs: a(|x| x.bold = true), ..Default::default() },
        Role::Bar => Style { fg: Some(Color::Green), ..Default::default() },
        Role::Header => Style { attrs: a(|x| { x.bold = true; x.underline = true }), ..Default::default() },
        Role::Status => Style { attrs: a(|x| x.reverse = true), ..Default::default() },
        Role::Normal | Role::Border => Style::default(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sheet() -> StyleSheet {
        let mut s = StyleSheet::default();
        s.palette.insert("STONE".into(), Color::Rgb(0x5a, 0x5a, 0x6e));
        s
    }

    #[test]
    fn hex_colors() {
        assert_eq!(parse_hex("5a5a6e"), Some(Color::Rgb(0x5a, 0x5a, 0x6e)));
        assert_eq!(parse_hex("5A5A6E"), None, "lower case only");
        assert_eq!(parse_hex("#5a5a6e"), None);
        assert_eq!(parse_hex("5a5a6"), None);
    }

    #[test]
    fn color_lookup_order() {
        let s = sheet();
        assert_eq!(s.color("STONE"), Some(Color::Rgb(0x5a, 0x5a, 0x6e)));
        assert_eq!(s.color("dark_grey"), Some(Color::DarkGrey));
        assert_eq!(s.color("ff0000"), Some(Color::Rgb(255, 0, 0)));
        assert_eq!(s.color("mauve"), None);
    }

    #[test]
    fn style_rows() {
        let s = sheet();
        let r = s.parse_row(&["border", "single", "fg", "STONE", "bg", "black", "bold"]).unwrap();
        assert_eq!(r.border, Some(Border::Single));
        assert_eq!(r.style.fg, Some(Color::Rgb(0x5a, 0x5a, 0x6e)));
        assert_eq!(r.style.bg, Some(Color::Black));
        assert!(r.style.attrs.bold);
        assert!(s.parse_row(&["fg"]).is_err());
        assert!(s.parse_row(&["fg", "mauve"]).unwrap_err().contains("unknown color"));
        assert!(s.parse_row(&["border", "wavy"]).is_err());
        assert!(s.parse_row(&["blink"]).unwrap_err().contains("unknown style word"));
    }

    #[test]
    fn resolution_layers_area_and_variant() {
        let mut s = sheet();
        s.rows.insert("LOG".into(), s.parse_row(&["fg", "white", "bg", "STONE"]).unwrap());
        s.rows.insert("LOG.stderr".into(), s.parse_row(&["fg", "yellow"]).unwrap());
        s.rows.insert("MENU.selected".into(), s.parse_row(&["fg", "black", "bg", "cyan"]).unwrap());
        let log = vec!["LOG".to_string()];
        assert_eq!(s.resolve(&log, Role::Normal).fg, Some(Color::White));
        assert_eq!(s.resolve(&log, Role::Stderr).fg, Some(Color::Yellow));
        assert_eq!(s.resolve(&log, Role::Stderr).bg, Some(Color::Rgb(0x5a, 0x5a, 0x6e)), "area background stays");
        let menu = vec!["MENU".to_string(), "MAIN_MENU".to_string()];
        let sel = s.resolve(&menu, Role::Selected);
        assert_eq!((sel.fg, sel.bg), (Some(Color::Black), Some(Color::Cyan)));
        assert!(sel.attrs.reverse, "built-in attributes remain");
        assert_eq!(s.resolve(&[], Role::Stderr).fg, Some(Color::Red), "built-in look");
    }

    #[test]
    fn borders() {
        let mut s = sheet();
        s.rows.insert("MENU".into(), s.parse_row(&["border", "double"]).unwrap());
        s.rows.insert("SETTINGS".into(), s.parse_row(&["border", "rounded"]).unwrap());
        assert_eq!(s.border(&["MENU".into()]), Some(Border::Double));
        assert_eq!(s.border(&["MENU".into(), "SETTINGS".into()]), Some(Border::Rounded));
        assert_eq!(Border::Ascii.chars().unwrap()[2], '+');
        assert!(Border::None.chars().is_none());
    }

    #[test]
    fn no_color() {
        let st = builtin(Role::Warning).without_colors();
        assert_eq!(st.fg, None);
        assert!(st.attrs.bold);
    }
}
