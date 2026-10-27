//! Runtime state of one menu: rows, cursor, filter and multi-selection
//! (spec §7, §7.1).

use std::collections::BTreeSet;

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Row {
    /// Item key (static menus) or the source value `$it` (lists).
    pub key: String,
    pub label: String,
    pub columns: Vec<String>,
    pub disabled: bool,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct MenuState {
    all: Vec<Row>,
    /// Indices into `all` that pass the filter.
    visible: Vec<usize>,
    pub cursor: usize,
    pub scroll: usize,
    pub filter: String,
    pub selected: BTreeSet<String>,
}

/// Case-insensitive subsequence match (fzf style).
pub fn fuzzy_match(pattern: &str, text: &str) -> bool {
    let mut t = text.chars().flat_map(char::to_lowercase);
    pattern.chars().flat_map(char::to_lowercase).all(|p| t.any(|c| c == p))
}

impl MenuState {
    pub fn new(rows: Vec<Row>) -> Self {
        let mut m = MenuState::default();
        m.set_rows(rows);
        m
    }

    /// Replaces the rows, keeping the cursor on the same key when possible.
    pub fn set_rows(&mut self, rows: Vec<Row>) {
        let current = self.current().map(|r| r.key.clone());
        self.all = rows;
        let keys: BTreeSet<&String> = self.all.iter().map(|r| &r.key).collect();
        self.selected.retain(|k| keys.contains(k));
        self.refilter();
        if let Some(k) = current
            && let Some(i) = self.visible.iter().position(|&i| self.all[i].key == k) {
                self.cursor = i;
            }
        self.clamp();
    }

    fn refilter(&mut self) {
        self.visible = (0..self.all.len()).filter(|&i| fuzzy_match(&self.filter, &self.all[i].label)).collect();
    }

    fn clamp(&mut self) {
        if self.visible.is_empty() {
            self.cursor = 0;
            return;
        }
        self.cursor = self.cursor.min(self.visible.len() - 1);
        if self.rows().nth(self.cursor).is_some_and(|r| r.disabled) {
            // move to the nearest enabled row, forwards first
            if !self.step(1) {
                self.step(-1);
            }
        }
    }

    pub fn rows(&self) -> impl Iterator<Item = &Row> {
        self.visible.iter().map(|&i| &self.all[i])
    }

    pub fn len(&self) -> usize {
        self.visible.len()
    }

    pub fn is_empty(&self) -> bool {
        self.visible.is_empty()
    }

    pub fn total(&self) -> usize {
        self.all.len()
    }

    pub fn current(&self) -> Option<&Row> {
        self.visible.get(self.cursor).map(|&i| &self.all[i])
    }

    /// Moves one enabled row in `dir` without wrapping; false if none.
    fn step(&mut self, dir: isize) -> bool {
        let mut i = self.cursor as isize + dir;
        while i >= 0 && (i as usize) < self.visible.len() {
            if !self.all[self.visible[i as usize]].disabled {
                self.cursor = i as usize;
                return true;
            }
            i += dir;
        }
        false
    }

    /// Moves `n` enabled rows; wraps around at the ends when `wrap`.
    pub fn move_by(&mut self, n: isize, wrap: bool) {
        let dir = n.signum();
        for _ in 0..n.unsigned_abs() {
            if !self.step(dir) && wrap {
                let start = self.cursor;
                self.cursor = if dir > 0 { 0 } else { self.visible.len().saturating_sub(1) };
                if self.current().is_some_and(|r| r.disabled) && !self.step(dir) {
                    self.cursor = start;
                }
                break;
            }
        }
    }

    pub fn home(&mut self) {
        self.cursor = 0;
        self.clamp();
    }

    pub fn end(&mut self) {
        self.cursor = self.visible.len().saturating_sub(1);
        if self.current().is_some_and(|r| r.disabled) {
            self.step(-1);
        }
    }

    /// Moves the cursor to visible row `i` if it is enabled.
    pub fn select_index(&mut self, i: usize) -> bool {
        match self.visible.get(i) {
            Some(&idx) if !self.all[idx].disabled => {
                self.cursor = i;
                true
            }
            _ => false,
        }
    }

    pub fn set_filter(&mut self, f: &str) {
        self.filter = f.to_string();
        self.refilter();
        self.cursor = 0;
        self.clamp();
    }

    pub fn push_filter(&mut self, c: char) {
        let f = format!("{}{c}", self.filter);
        self.set_filter(&f);
    }

    pub fn pop_filter(&mut self) -> bool {
        let mut f = self.filter.clone();
        let popped = f.pop().is_some();
        self.set_filter(&f);
        popped
    }

    /// Toggles multi-selection of the current row.
    pub fn toggle_selected(&mut self) {
        if let Some(k) = self.current().filter(|r| !r.disabled).map(|r| r.key.clone())
            && !self.selected.remove(&k) {
                self.selected.insert(k);
            }
    }

    /// Selected keys in row order (the current row when nothing is marked).
    pub fn selection(&self) -> Vec<String> {
        if self.selected.is_empty() {
            return self.current().filter(|r| !r.disabled).map(|r| vec![r.key.clone()]).unwrap_or_default();
        }
        self.all.iter().filter(|r| self.selected.contains(&r.key)).map(|r| r.key.clone()).collect()
    }

    /// Keeps the cursor inside a window of `height` rows.
    pub fn scroll_into_view(&mut self, height: usize) {
        if height == 0 {
            return;
        }
        if self.cursor < self.scroll {
            self.scroll = self.cursor;
        } else if self.cursor >= self.scroll + height {
            self.scroll = self.cursor + 1 - height;
        }
        self.scroll = self.scroll.min(self.visible.len().saturating_sub(height));
    }
}

/// Letter shortcut for visible row `i` (a–z, then none).
pub fn row_letter(i: usize) -> Option<char> {
    (i < 26).then(|| (b'a' + i as u8) as char)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rows(spec: &[(&str, bool)]) -> Vec<Row> {
        spec.iter().map(|(k, d)| Row { key: k.to_string(), label: k.to_string(), columns: vec![], disabled: *d }).collect()
    }

    #[test]
    fn cursor_skips_disabled_rows() {
        let mut m = MenuState::new(rows(&[("a", false), ("b", true), ("c", false)]));
        assert_eq!(m.current().unwrap().key, "a");
        m.move_by(1, false);
        assert_eq!(m.current().unwrap().key, "c");
        m.move_by(1, false);
        assert_eq!(m.current().unwrap().key, "c", "no wrap");
        m.move_by(1, true);
        assert_eq!(m.current().unwrap().key, "a", "wraps");
        m.move_by(-1, true);
        assert_eq!(m.current().unwrap().key, "c");
    }

    #[test]
    fn first_enabled_row_selected_initially() {
        let m = MenuState::new(rows(&[("a", true), ("b", false)]));
        assert_eq!(m.current().unwrap().key, "b");
    }

    #[test]
    fn all_disabled_menu_has_no_usable_row() {
        let m = MenuState::new(rows(&[("a", true)]));
        assert!(m.selection().is_empty());
    }

    #[test]
    fn home_end() {
        let mut m = MenuState::new(rows(&[("a", true), ("b", false), ("c", false), ("d", true)]));
        m.end();
        assert_eq!(m.current().unwrap().key, "c");
        m.home();
        assert_eq!(m.current().unwrap().key, "b");
    }

    #[test]
    fn set_rows_keeps_cursor_on_key() {
        let mut m = MenuState::new(rows(&[("a", false), ("b", false), ("c", false)]));
        m.move_by(2, false);
        m.set_rows(rows(&[("x", false), ("c", false)]));
        assert_eq!(m.current().unwrap().key, "c");
        m.set_rows(rows(&[("x", false)]));
        assert_eq!(m.current().unwrap().key, "x");
    }

    #[test]
    fn row_becoming_disabled_moves_cursor() {
        let mut m = MenuState::new(rows(&[("a", false), ("b", false)]));
        m.set_rows(rows(&[("a", true), ("b", false)]));
        assert_eq!(m.current().unwrap().key, "b");
    }

    #[test]
    fn fuzzy() {
        assert!(fuzzy_match("sys", "/var/log/syslog"));
        assert!(fuzzy_match("SL", "syslog"));
        assert!(fuzzy_match("", "x"));
        assert!(!fuzzy_match("xyz", "syslog"));
        assert!(!fuzzy_match("gs", "syslog"), "order matters");
    }

    #[test]
    fn filtering() {
        let mut m = MenuState::new(rows(&[("auth.log", false), ("syslog", false), ("kern.log", false)]));
        m.push_filter('s');
        m.push_filter('y');
        assert_eq!(m.len(), 1);
        assert_eq!(m.total(), 3);
        assert_eq!(m.current().unwrap().key, "syslog");
        assert!(m.pop_filter());
        assert_eq!(m.filter, "s");
        m.set_filter("zzz");
        assert!(m.is_empty());
        assert!(m.current().is_none());
    }

    #[test]
    fn multi_selection_in_row_order() {
        let mut m = MenuState::new(rows(&[("a", false), ("b", false), ("c", false)]));
        assert_eq!(m.selection(), ["a"], "current row when nothing marked");
        m.move_by(2, false);
        m.toggle_selected();
        m.home();
        m.toggle_selected();
        assert_eq!(m.selection(), ["a", "c"]);
        m.toggle_selected();
        assert_eq!(m.selection(), ["c"]);
    }

    #[test]
    fn select_index_respects_disabled() {
        let mut m = MenuState::new(rows(&[("a", false), ("b", true)]));
        assert!(!m.select_index(1));
        assert!(!m.select_index(5));
        assert!(m.select_index(0));
    }

    #[test]
    fn scrolling() {
        let mut m = MenuState::new(rows(&[("a", false), ("b", false), ("c", false), ("d", false), ("e", false)]));
        m.move_by(4, false);
        m.scroll_into_view(2);
        assert_eq!(m.scroll, 3);
        m.home();
        m.scroll_into_view(2);
        assert_eq!(m.scroll, 0);
    }

    #[test]
    fn letters() {
        assert_eq!(row_letter(0), Some('a'));
        assert_eq!(row_letter(25), Some('z'));
        assert_eq!(row_letter(26), None);
    }
}
