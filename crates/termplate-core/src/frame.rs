//! The drawing model: a grid of characters with style roles. The terminal
//! layer maps roles to colors (spec §11) and draws only what changed.

use crate::layout::Rect;
use crate::model::Align;
use crate::style::{Border, Style};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Hash, PartialOrd, Ord)]
pub enum Role {
    #[default]
    Normal,
    Border,
    Title,
    Field,
    Selected,
    Disabled,
    Dim,
    Input,
    Stderr,
    Warning,
    Bar,
    Header,
    Status,
}

impl Role {
    pub fn name(self) -> &'static str {
        match self {
            Role::Normal => "normal",
            Role::Border => "border",
            Role::Title => "title",
            Role::Field => "field",
            Role::Selected => "selected",
            Role::Disabled => "disabled",
            Role::Dim => "dim",
            Role::Input => "input",
            Role::Stderr => "stderr",
            Role::Warning => "warning",
            Role::Bar => "bar",
            Role::Header => "header",
            Role::Status => "status",
        }
    }
}

/// The cell after a two-column character holds this instead of a
/// character; the terminal skips it.
pub const WIDE_CONT: char = '\0';

/// Display width of a character: 0 for combining marks and controls, 2 for
/// wide (CJK, emoji) characters.
pub fn char_width(c: char) -> usize {
    unicode_width::UnicodeWidthChar::width(c).unwrap_or(0)
}

/// Display width of a string in terminal columns.
pub fn str_width(s: &str) -> usize {
    s.chars().map(char_width).sum()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Cell {
    pub ch: char,
    pub role: Role,
    /// Index into [`Frame::areas`]: the area chain whose style applies.
    pub area: u16,
    /// The cell's own style over the role's (ANSI colors in a log).
    pub style: Option<Style>,
}

impl Default for Cell {
    fn default() -> Self {
        Cell { ch: ' ', role: Role::Normal, area: 0, style: None }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Frame {
    pub width: usize,
    pub height: usize,
    pub cells: Vec<Cell>,
    /// Text cursor position for input fields.
    pub cursor: Option<(usize, usize)>,
    /// Area chains for styling (`["MENU", "MAIN_MENU"]`); index 0 is empty.
    pub areas: Vec<Vec<String>>,
    current: u16,
}

impl Frame {
    pub fn new(width: usize, height: usize) -> Self {
        Frame { width, height, cells: vec![Cell::default(); width * height], cursor: None, areas: vec![vec![]], current: 0 }
    }

    /// Cells drawn from now on belong to the area chain `chain`.
    pub fn set_area(&mut self, chain: Vec<String>) {
        self.current = match self.areas.iter().position(|a| *a == chain) {
            Some(i) => i as u16,
            None => {
                self.areas.push(chain);
                (self.areas.len() - 1) as u16
            }
        };
    }

    pub fn area_of(&self, cell: &Cell) -> &[String] {
        self.areas.get(cell.area as usize).map(Vec::as_slice).unwrap_or(&[])
    }

    pub fn get(&self, x: usize, y: usize) -> Option<&Cell> {
        (x < self.width && y < self.height).then(|| &self.cells[y * self.width + x])
    }

    pub fn put(&mut self, x: usize, y: usize, ch: char, role: Role) {
        if x < self.width && y < self.height {
            let i = y * self.width + x;
            // Overwriting half of a wide character blanks the other half.
            if self.cells[i].ch == WIDE_CONT && x > 0 {
                self.cells[i - 1].ch = ' ';
            }
            if x + 1 < self.width && self.cells[i + 1].ch == WIDE_CONT && ch != WIDE_CONT {
                self.cells[i + 1].ch = ' ';
            }
            self.cells[i] = Cell { ch, role, area: self.current, style: None };
        }
    }

    pub fn fill(&mut self, r: Rect, ch: char, role: Role) {
        for y in r.y..r.y + r.h {
            for x in r.x..r.x + r.w {
                self.put(x, y, ch, role);
            }
        }
    }

    /// Writes `s` at (x, y), clipped to `max` columns. A wide character
    /// takes two cells; one that does not fit is left out. Returns the
    /// columns written.
    pub fn text(&mut self, x: usize, y: usize, max: usize, s: &str, role: Role) -> usize {
        let mut n = 0;
        for ch in s.chars() {
            let ch = if ch.is_control() { ' ' } else { ch };
            let w = char_width(ch);
            if w == 0 {
                continue;
            }
            if n + w > max {
                break;
            }
            self.put(x + n, y, ch, role);
            if w == 2 {
                self.put(x + n + 1, y, WIDE_CONT, role);
            }
            n += w;
        }
        n
    }

    /// Writes `s` aligned within a one-line area of width `w`.
    /// Like [`Frame::text`], with a style per character of `s` over `role`.
    pub fn styled_text(&mut self, x: usize, y: usize, max: usize, s: &str, role: Role, styles: &[Style]) -> usize {
        let mut n = 0;
        for (ch, style) in s.chars().zip(styles.iter().chain(std::iter::repeat(&Style::default()))) {
            let w = self.text(x + n, y, max - n, &ch.to_string(), role);
            if w == 0 && char_width(ch) > 0 {
                break;
            }
            if *style != Style::default() {
                for c in x + n..x + n + w {
                    if let Some(cell) = (y < self.height && c < self.width).then(|| &mut self.cells[y * self.width + c]) {
                        cell.style = Some(*style);
                    }
                }
            }
            n += w;
        }
        n
    }

    pub fn aligned(&mut self, x: usize, y: usize, w: usize, s: &str, align: Align, role: Role) {
        let s = fit(s, w, align);
        self.text(x, y, w, &s, role);
    }

    /// Draws a box border on the edge of `r`.
    pub fn border(&mut self, r: Rect, border: Border, role: Role) {
        let Some([h, v, tl, tr, bl, br]) = border.chars() else { return };
        if r.w < 2 || r.h < 2 {
            return;
        }
        let (x1, y1) = (r.x + r.w - 1, r.y + r.h - 1);
        for x in r.x + 1..x1 {
            self.put(x, r.y, h, role);
            self.put(x, y1, h, role);
        }
        for y in r.y + 1..y1 {
            self.put(r.x, y, v, role);
            self.put(x1, y, v, role);
        }
        self.put(r.x, r.y, tl, role);
        self.put(x1, r.y, tr, role);
        self.put(r.x, y1, bl, role);
        self.put(x1, y1, br, role);
    }

    /// Row text (for tests and plain output).
    pub fn row(&self, y: usize) -> String {
        (0..self.width).map(|x| self.cells[y * self.width + x].ch).filter(|c| *c != WIDE_CONT).collect()
    }

    pub fn rows(&self) -> Vec<String> {
        (0..self.height).map(|y| self.row(y)).collect()
    }

    /// Positions that differ from `prev` (all positions when sizes differ).
    pub fn diff(&self, prev: &Frame) -> Vec<(usize, usize)> {
        if prev.width != self.width || prev.height != self.height {
            return (0..self.height).flat_map(|y| (0..self.width).map(move |x| (x, y))).collect();
        }
        self.cells
            .iter()
            .zip(&prev.cells)
            .enumerate()
            .filter(|(_, (a, b))| a != b)
            .map(|(i, _)| (i % self.width, i / self.width))
            .collect()
    }
}

/// The longest start of `s` that is at most `w` columns wide.
pub fn take_width(s: &str, w: usize) -> &str {
    let mut used = 0;
    for (i, c) in s.char_indices() {
        used += char_width(c);
        if used > w {
            return &s[..i];
        }
    }
    s
}

/// Pads `s` to exactly `w` columns with the given alignment; text that
/// does not fit is cut with `…` (D 11.5).
pub fn fit(s: &str, w: usize, align: Align) -> String {
    let len = str_width(s);
    if len > w {
        if w == 0 {
            return String::new();
        }
        let mut t = take_width(s, w - 1).to_string();
        t.push('…');
        // a cut wide character leaves one column
        let short = w - str_width(&t);
        t.push_str(&" ".repeat(short));
        return t;
    }
    if len == w {
        return s.to_string();
    }
    let pad = w - len;
    let (l, r) = match align {
        Align::Left => (0, pad),
        Align::Right => (pad, 0),
        Align::Center => (pad / 2, pad - pad / 2),
    };
    format!("{}{s}{}", " ".repeat(l), " ".repeat(r))
}

/// Wraps `text` to lines of at most `width` columns, breaking between
/// words (D 11.4, D 11.5); a word longer than a line is split. A line that
/// fits stays as it is, spaces included, so aligned columns survive.
pub fn wrap(text: &str, width: usize) -> Vec<String> {
    let mut lines = Vec::new();
    if width == 0 {
        return lines;
    }
    for para in text.split('\n') {
        let para = para.trim_end();
        if str_width(para) <= width {
            lines.push(para.to_string());
            continue;
        }
        let mut line = String::new();
        let mut len = 0;
        for word in para.split(' ').filter(|w| !w.is_empty()) {
            let mut word = word;
            if len > 0 && len + 1 + str_width(word) > width {
                lines.push(std::mem::take(&mut line));
                len = 0;
            }
            while str_width(word) > width {
                if len > 0 {
                    lines.push(std::mem::take(&mut line));
                    len = 0;
                }
                let mut head = take_width(word, width);
                if head.is_empty() {
                    // a wide character in a one-column line
                    head = &word[..word.chars().next().map_or(0, char::len_utf8)];
                }
                lines.push(head.to_string());
                word = &word[head.len()..];
            }
            if word.is_empty() {
                continue;
            }
            if len > 0 {
                line.push(' ');
                len += 1;
            }
            line.push_str(word);
            len += str_width(word);
        }
        lines.push(line);
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fit_aligns_and_truncates() {
        assert_eq!(fit("ab", 5, Align::Left), "ab   ");
        assert_eq!(fit("ab", 5, Align::Right), "   ab");
        assert_eq!(fit("ab", 5, Align::Center), " ab  ");
        assert_eq!(fit("abcdef", 3, Align::Right), "ab…");
        assert_eq!(fit("abc", 3, Align::Right), "abc");
        assert_eq!(fit("abc", 0, Align::Left), "");
        assert_eq!(fit("äö", 3, Align::Left), "äö ");
    }

    #[test]
    fn wrapping() {
        assert_eq!(wrap("the quick brown fox", 9), ["the quick", "brown fox"]);
        assert_eq!(wrap("a\nb c", 10), ["a", "b c"]);
        assert_eq!(wrap("abcdefghij", 4), ["abcd", "efgh", "ij"]);
        assert_eq!(wrap("ab abcdefgh", 4), ["ab", "abcd", "efgh"]);
        assert_eq!(wrap("", 4), [""]);
        assert_eq!(wrap(" 5 3 .  │ 7  \nx", 12), [" 5 3 .  │ 7", "x"], "lines that fit keep their spacing");
        assert!(wrap("x", 0).is_empty());
    }

    #[test]
    fn wide_characters_take_two_columns() {
        assert_eq!(str_width("日本"), 4);
        assert_eq!(fit("日本語", 4, Align::Left), "日… ");
        assert_eq!(fit("日本", 6, Align::Right), "  日本");
        assert_eq!(wrap("日本語 です", 6), ["日本語", "です"]);
        assert_eq!(wrap("日本語", 4), ["日本", "語"]);
        let mut f = Frame::new(6, 1);
        assert_eq!(f.text(0, 0, 6, "a日b", Role::Normal), 4);
        assert_eq!(f.row(0), "a日b  ");
        assert_eq!(f.get(2, 0).unwrap().ch, WIDE_CONT);
        assert_eq!(f.text(0, 0, 2, "a日", Role::Normal), 1, "a wide char that does not fit is left out");
        f.put(1, 0, 'x', Role::Normal);
        assert_eq!(f.row(0), "ax b  ", "overwriting a wide char blanks its other half");
    }

    #[test]
    fn text_is_clipped_to_frame_and_max() {
        let mut f = Frame::new(5, 1);
        assert_eq!(f.text(3, 0, 10, "hello", Role::Normal), 5);
        assert_eq!(f.row(0), "   he");
        assert_eq!(f.text(0, 0, 2, "xyz", Role::Normal), 2);
        assert_eq!(f.row(0), "xy he");
    }

    #[test]
    fn control_chars_become_spaces() {
        let mut f = Frame::new(3, 1);
        f.text(0, 0, 3, "a\tb", Role::Normal);
        assert_eq!(f.row(0), "a b");
    }

    #[test]
    fn border_draws_box() {
        let mut f = Frame::new(4, 3);
        f.border(Rect::new(0, 0, 4, 3), Border::Single, Role::Border);
        assert_eq!(f.rows(), ["┌──┐", "│  │", "└──┘"]);
        assert_eq!(f.get(0, 0).unwrap().role, Role::Border);
        f.border(Rect::new(0, 0, 4, 3), Border::Ascii, Role::Border);
        assert_eq!(f.row(0), "+--+");
        f.border(Rect::new(0, 0, 4, 3), Border::None, Role::Border);
        assert_eq!(f.row(0), "+--+", "none draws nothing");
    }

    #[test]
    fn cells_remember_their_area() {
        let mut f = Frame::new(3, 1);
        f.put(0, 0, 'a', Role::Normal);
        f.set_area(vec!["LOG".into()]);
        f.put(1, 0, 'b', Role::Normal);
        f.set_area(vec![]);
        f.put(2, 0, 'c', Role::Normal);
        assert_eq!(f.area_of(f.get(0, 0).unwrap()), &[] as &[String]);
        assert_eq!(f.area_of(f.get(1, 0).unwrap()), ["LOG".to_string()]);
        assert_eq!(f.get(2, 0).unwrap().area, 0);
    }

    #[test]
    fn diff_reports_changed_cells() {
        let a = Frame::new(3, 2);
        let mut b = a.clone();
        b.put(2, 1, 'x', Role::Normal);
        assert_eq!(b.diff(&a), [(2, 1)]);
        assert_eq!(Frame::new(2, 1).diff(&a).len(), 2);
    }

    #[test]
    fn fill_and_get() {
        let mut f = Frame::new(3, 3);
        f.fill(Rect::new(1, 1, 5, 5), '#', Role::Bar);
        assert_eq!(f.rows(), ["   ", " ##", " ##"]);
        assert!(f.get(3, 0).is_none());
    }
}
