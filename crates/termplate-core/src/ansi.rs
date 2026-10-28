//! ANSI colors in log lines (`log NAME ansi keep`, spec §12).
//!
//! SGR sequences (`ESC [ … m`) become a style per character: the 8 basic
//! and 8 bright colors, the 256-color palette, 24-bit colors, and bold,
//! dim, italic, underline and reverse with their resets. Other escape
//! sequences (cursor movement, OSC titles) are dropped.

use crate::style::{Color, Style};

/// Basic colors 0–7 and their bright versions 8–15, in crossterm's naming
/// (`Red` is the bright one).
const BASIC: [Color; 16] = [
    Color::Black,
    Color::DarkRed,
    Color::DarkGreen,
    Color::DarkYellow,
    Color::DarkBlue,
    Color::DarkMagenta,
    Color::DarkCyan,
    Color::Grey,
    Color::DarkGrey,
    Color::Red,
    Color::Green,
    Color::Yellow,
    Color::Blue,
    Color::Magenta,
    Color::Cyan,
    Color::White,
];

fn palette(n: u16) -> Option<Color> {
    match n {
        0..=15 => Some(BASIC[n as usize]),
        16..=255 => Some(Color::Ansi(n as u8)),
        _ => None,
    }
}

/// `38;5;N` or `38;2;R;G;B` from `params[i..]` (after the 38/48); returns
/// the color and how many parameters it took.
fn extended(params: &[u16]) -> (Option<Color>, usize) {
    match params {
        [5, n, ..] => (palette(*n), 2),
        [2, r, g, b, ..] => (Some(Color::Rgb(*r as u8, *g as u8, *b as u8)), 4),
        [5] | [2, ..] => (None, params.len()),
        _ => (None, 0),
    }
}

fn apply(style: &mut Style, params: &[u16]) {
    if params.is_empty() {
        *style = Style::default();
        return;
    }
    let mut i = 0;
    while i < params.len() {
        let p = params[i];
        i += 1;
        let a = &mut style.attrs;
        match p {
            0 => *style = Style::default(),
            1 => a.bold = true,
            2 => a.dim = true,
            3 => a.italic = true,
            4 => a.underline = true,
            7 => a.reverse = true,
            22 => (a.bold, a.dim) = (false, false),
            23 => a.italic = false,
            24 => a.underline = false,
            27 => a.reverse = false,
            30..=37 => style.fg = Some(BASIC[(p - 30) as usize]),
            90..=97 => style.fg = Some(BASIC[(p - 90 + 8) as usize]),
            40..=47 => style.bg = Some(BASIC[(p - 40) as usize]),
            100..=107 => style.bg = Some(BASIC[(p - 100 + 8) as usize]),
            39 => style.fg = None,
            49 => style.bg = None,
            38 | 48 => {
                let (c, used) = extended(&params[i..]);
                i += used;
                if p == 38 {
                    style.fg = c.or(style.fg);
                } else {
                    style.bg = c.or(style.bg);
                }
            }
            _ => {}
        }
    }
}

/// The text without escape sequences and the style of each of its
/// characters; `None` when nothing is styled.
pub fn parse(s: &str) -> (String, Option<Vec<Style>>) {
    let mut text = String::with_capacity(s.len());
    let mut styles = Vec::new();
    let mut style = Style::default();
    let mut styled = false;
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '\x1b' {
            text.push(c);
            styles.push(style);
            styled |= style != Style::default();
            continue;
        }
        match chars.next() {
            Some('[') => {
                let mut body = String::new();
                let mut fin = None;
                for c in chars.by_ref() {
                    if ('@'..='~').contains(&c) {
                        fin = Some(c);
                        break;
                    }
                    body.push(c);
                }
                if fin == Some('m') {
                    let params: Vec<u16> = body.split([';', ':']).map(|p| p.parse().unwrap_or(0)).collect();
                    let params = if body.is_empty() { vec![] } else { params };
                    apply(&mut style, &params);
                }
            }
            Some(']') => {
                while let Some(c) = chars.next() {
                    if c == '\x07' {
                        break;
                    }
                    if c == '\x1b' && chars.peek() == Some(&'\\') {
                        chars.next();
                        break;
                    }
                }
            }
            _ => {}
        }
    }
    (text, styled.then_some(styles))
}

/// Styles for the lines `wrap` made of a text with `styles` per character:
/// the lines' characters are the text's in order, minus spaces at breaks;
/// added characters (`…`) get no style.
pub fn wrap_styles(lines: &[String], text: &str, styles: &[Style]) -> Vec<Vec<Style>> {
    let src: Vec<char> = text.chars().collect();
    let mut at = 0;
    lines
        .iter()
        .map(|line| {
            line.chars()
                .map(|c| match src[at..].iter().position(|s| *s == c) {
                    Some(off) => {
                        at += off + 1;
                        styles.get(at - 1).copied().unwrap_or_default()
                    }
                    None => Style::default(),
                })
                .collect()
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::style::Attrs;

    fn fg(c: Color) -> Style {
        Style { fg: Some(c), ..Default::default() }
    }

    #[test]
    fn plain_text_has_no_styles() {
        assert_eq!(parse("hello"), ("hello".into(), None));
        assert_eq!(parse("a\x1b[2Kb\x1b]0;title\x07c"), ("abc".into(), None), "non-SGR sequences are dropped");
    }

    #[test]
    fn basic_bright_and_reset() {
        let (t, s) = parse("\x1b[31mE\x1b[0m \x1b[1;92mok\x1b[m!");
        assert_eq!(t, "E ok!");
        let s = s.unwrap();
        assert_eq!(s[0], fg(Color::DarkRed));
        assert_eq!(s[1], Style::default());
        assert_eq!(s[2], Style { fg: Some(Color::Green), attrs: Attrs { bold: true, ..Default::default() }, ..Default::default() });
        assert_eq!(s[4], Style::default(), "`ESC[m` resets");
    }

    #[test]
    fn extended_colors_and_attribute_resets() {
        let (_, s) = parse("\x1b[38;5;208;48;2;1;2;3;4ma\x1b[24;39mb\x1b[38;5;3mc");
        let s = s.unwrap();
        assert_eq!(s[0].fg, Some(Color::Ansi(208)));
        assert_eq!(s[0].bg, Some(Color::Rgb(1, 2, 3)));
        assert!(s[0].attrs.underline);
        assert_eq!(s[1], Style { bg: Some(Color::Rgb(1, 2, 3)), ..Default::default() });
        assert_eq!(s[2].fg, Some(Color::DarkYellow), "palette 0-15 are the basic colors");
    }

    #[test]
    fn styles_follow_wrapped_lines() {
        let (t, s) = parse("ab \x1b[31mcd\x1b[0m  ef");
        let s = s.unwrap();
        let lines = vec!["ab".to_string(), "cd ef".to_string(), "…".to_string()];
        let w = wrap_styles(&lines, &t, &s);
        assert_eq!(w[0], [Style::default(); 2]);
        assert_eq!(w[1][..2], [fg(Color::DarkRed); 2]);
        assert_eq!(w[1][2..], [Style::default(); 3]);
        assert_eq!(w[2], [Style::default()]);
    }
}
