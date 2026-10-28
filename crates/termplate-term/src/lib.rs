//! Terminal drawing and input with crossterm (spec §16 `termplate-term`).
//!
//! * [`Screen`] draws [`Frame`]s, writing only the cells that changed. Each
//!   cell's look comes from the application's style sheet (RLML D 9) via
//!   its area chain and role; `NO_COLOR` keeps attributes only.
//! * [`TtyGuard`] puts `/dev/tty` into raw mode and the alternate screen and
//!   restores it on drop, on panic and via [`restore`] (for signals).
//! * [`map_event`] turns crossterm events into core input events.

use std::fs::{File, OpenOptions};
use std::io::{self, Write};

use crossterm::event::{
    DisableMouseCapture, EnableMouseCapture, Event as CEvent, KeyCode as CKey, KeyEventKind, KeyModifiers, MouseButton,
    MouseEventKind,
};
use crossterm::style::{Attribute, Color as CColor, Print, ResetColor, SetAttribute, SetBackgroundColor, SetForegroundColor};
use crossterm::{cursor, queue, terminal};
use termplate_core::engine::InputEvent;
use termplate_core::frame::Frame;
use termplate_core::keys::{Binding, KeyCode, Mods, Mouse};
use termplate_core::style::{Color, Style, StyleSheet};

pub fn to_crossterm(c: Color) -> CColor {
    match c {
        Color::Reset => CColor::Reset,
        Color::Black => CColor::Black,
        Color::Red => CColor::Red,
        Color::Green => CColor::Green,
        Color::Yellow => CColor::Yellow,
        Color::Blue => CColor::Blue,
        Color::Magenta => CColor::Magenta,
        Color::Cyan => CColor::Cyan,
        Color::White => CColor::White,
        Color::Grey => CColor::Grey,
        Color::DarkRed => CColor::DarkRed,
        Color::DarkGreen => CColor::DarkGreen,
        Color::DarkYellow => CColor::DarkYellow,
        Color::DarkBlue => CColor::DarkBlue,
        Color::DarkMagenta => CColor::DarkMagenta,
        Color::DarkCyan => CColor::DarkCyan,
        Color::DarkGrey => CColor::DarkGrey,
        Color::Ansi(n) => CColor::AnsiValue(n),
        Color::Rgb(r, g, b) => CColor::Rgb { r, g, b },
    }
}

/// `NO_COLOR` set to any non-empty value disables color (no-color.org).
pub fn no_color_from_env(value: Option<&str>) -> bool {
    value.is_some_and(|v| !v.is_empty())
}

/// Draws frames to a writer, sending only changed cells.
pub struct Screen<W: Write> {
    pub out: W,
    pub sheet: StyleSheet,
    pub no_color: bool,
    prev: Option<Frame>,
}

impl<W: Write> Screen<W> {
    pub fn new(out: W, sheet: StyleSheet, no_color: bool) -> Self {
        Screen { out, sheet, no_color, prev: None }
    }

    /// Forces a full redraw on the next [`Screen::draw`].
    pub fn invalidate(&mut self) {
        self.prev = None;
    }

    fn style_of(&self, frame: &Frame, x: usize, y: usize) -> (char, Style) {
        let cell = frame.get(x, y).copied().unwrap_or_default();
        let mut style = self.sheet.resolve(frame.area_of(&cell), cell.role);
        if let Some(own) = cell.style {
            style = style.over(own);
        }
        (cell.ch, if self.no_color { style.without_colors() } else { style })
    }

    pub fn draw(&mut self, frame: &Frame) -> io::Result<()> {
        let full = self.prev.as_ref().is_none_or(|p| p.width != frame.width || p.height != frame.height);
        let changed: Vec<(usize, usize)> = match (&self.prev, full) {
            // a cell changed when its character, role or area chain did
            (Some(prev), false) => (0..frame.height)
                .flat_map(|y| (0..frame.width).map(move |x| (x, y)))
                .filter(|&(x, y)| match (frame.get(x, y), prev.get(x, y)) {
                    (Some(a), Some(b)) => a.ch != b.ch || a.role != b.role || a.style != b.style || frame.area_of(a) != prev.area_of(b),
                    _ => true,
                })
                .collect(),
            _ => {
                queue!(self.out, ResetColor, SetAttribute(Attribute::Reset), terminal::Clear(terminal::ClearType::All))?;
                (0..frame.height).flat_map(|y| (0..frame.width).map(move |x| (x, y))).collect()
            }
        };
        queue!(self.out, cursor::Hide)?;
        let mut last: Option<(usize, usize, Style)> = None;
        for (x, y) in changed {
            let (ch, style) = self.style_of(frame, x, y);
            if ch == termplate_core::frame::WIDE_CONT {
                // the wide character before it covers this column
                continue;
            }
            if !last.is_some_and(|(lx, ly, _)| ly == y && lx + 1 == x) {
                queue!(self.out, cursor::MoveTo(x as u16, y as u16))?;
            }
            if last.is_none_or(|(_, _, s)| s != style) {
                queue!(self.out, ResetColor, SetAttribute(Attribute::Reset))?;
                if let Some(fg) = style.fg {
                    queue!(self.out, SetForegroundColor(to_crossterm(fg)))?;
                }
                if let Some(bg) = style.bg {
                    queue!(self.out, SetBackgroundColor(to_crossterm(bg)))?;
                }
                let a = style.attrs;
                for (on, attr) in [
                    (a.bold, Attribute::Bold),
                    (a.dim, Attribute::Dim),
                    (a.italic, Attribute::Italic),
                    (a.underline, Attribute::Underlined),
                    (a.reverse, Attribute::Reverse),
                ] {
                    if on {
                        queue!(self.out, SetAttribute(attr))?;
                    }
                }
            }
            queue!(self.out, Print(ch))?;
            last = Some((x, y, style));
        }
        queue!(self.out, ResetColor, SetAttribute(Attribute::Reset))?;
        if let Some((x, y)) = frame.cursor {
            queue!(self.out, cursor::MoveTo(x as u16, y as u16), cursor::Show)?;
        }
        self.out.flush()?;
        self.prev = Some(frame.clone());
        Ok(())
    }
}

pub fn open_tty() -> io::Result<File> {
    OpenOptions::new().read(true).write(true).open("/dev/tty")
}

fn enter(out: &mut impl Write, mouse: bool) -> io::Result<()> {
    terminal::enable_raw_mode()?;
    queue!(out, terminal::EnterAlternateScreen, cursor::Hide)?;
    if mouse {
        queue!(out, EnableMouseCapture)?;
    }
    out.flush()
}

/// Restores the terminal. Safe to call more than once.
pub fn restore() {
    if let Ok(mut tty) = open_tty() {
        let _ = queue!(tty, DisableMouseCapture, ResetColor, cursor::Show, terminal::LeaveAlternateScreen);
        let _ = tty.flush();
    }
    let _ = terminal::disable_raw_mode();
}

/// Raw mode + alternate screen on `/dev/tty` for its lifetime.
pub struct TtyGuard {
    mouse: bool,
}

impl TtyGuard {
    /// `mouse` enables mouse reporting (settings `mouse on|off`, D 3.2).
    pub fn new(mouse: bool) -> io::Result<(TtyGuard, File)> {
        let mut tty = open_tty()?;
        enter(&mut tty, mouse)?;
        let prev = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            restore();
            prev(info);
        }));
        Ok((TtyGuard { mouse }, tty))
    }

    /// Gives the terminal to another program (interactive commands).
    pub fn suspend() {
        restore();
    }

    /// Takes the terminal back after [`TtyGuard::suspend`].
    pub fn resume(mouse: bool) -> io::Result<()> {
        let mut tty = open_tty()?;
        enter(&mut tty, mouse)
    }

    pub fn mouse(&self) -> bool {
        self.mouse
    }
}

impl Drop for TtyGuard {
    fn drop(&mut self) {
        restore();
    }
}

/// Terminal size; a terminal reporting 0×0 (some pseudo-terminals) falls
/// back to `COLUMNS`/`LINES`, then 80×24.
pub fn terminal_size() -> (usize, usize) {
    let env = |k: &str| std::env::var(k).ok();
    size_or_fallback(terminal::size().ok(), env("COLUMNS").as_deref(), env("LINES").as_deref())
}

pub fn size_or_fallback(size: Option<(u16, u16)>, columns: Option<&str>, lines: Option<&str>) -> (usize, usize) {
    match size {
        Some((w, h)) if w > 0 && h > 0 => (w as usize, h as usize),
        _ => (
            columns.and_then(|c| c.parse().ok()).filter(|&c: &usize| c > 0).unwrap_or(80),
            lines.and_then(|l| l.parse().ok()).filter(|&l: &usize| l > 0).unwrap_or(24),
        ),
    }
}

fn map_key(code: CKey) -> Option<KeyCode> {
    Some(match code {
        CKey::Char(c) => KeyCode::Char(c),
        CKey::Enter => KeyCode::Enter,
        CKey::Esc => KeyCode::Esc,
        CKey::Tab => KeyCode::Tab,
        CKey::BackTab => KeyCode::BackTab,
        CKey::Backspace => KeyCode::Backspace,
        CKey::Delete => KeyCode::Delete,
        CKey::Insert => KeyCode::Insert,
        CKey::Up => KeyCode::Up,
        CKey::Down => KeyCode::Down,
        CKey::Left => KeyCode::Left,
        CKey::Right => KeyCode::Right,
        CKey::Home => KeyCode::Home,
        CKey::End => KeyCode::End,
        CKey::PageUp => KeyCode::PageUp,
        CKey::PageDown => KeyCode::PageDown,
        CKey::F(n) => KeyCode::F(n),
        _ => return None,
    })
}

fn mods_of(m: KeyModifiers) -> Mods {
    Mods { ctrl: m.contains(KeyModifiers::CONTROL), alt: m.contains(KeyModifiers::ALT), shift: m.contains(KeyModifiers::SHIFT) }
}

pub fn map_event(ev: CEvent) -> Option<InputEvent> {
    match ev {
        CEvent::Key(k) if k.kind != KeyEventKind::Release => Some(InputEvent::Key(map_key(k.code)?, mods_of(k.modifiers))),
        CEvent::Mouse(m) => {
            let (x, y) = (m.column as usize, m.row as usize);
            let kind = match m.kind {
                MouseEventKind::Down(MouseButton::Left) => Mouse::Click,
                MouseEventKind::Down(MouseButton::Right) => Mouse::RightClick,
                MouseEventKind::Down(MouseButton::Middle) => Mouse::MiddleClick,
                MouseEventKind::ScrollUp => Mouse::WheelUp,
                MouseEventKind::ScrollDown => Mouse::WheelDown,
                MouseEventKind::Moved => return Some(InputEvent::MouseMove { x, y }),
                _ => return None,
            };
            Some(InputEvent::Mouse { binding: Binding::Mouse(kind, mods_of(m.modifiers)), x, y })
        }
        CEvent::Resize(w, h) => Some(InputEvent::Resize(w as usize, h as usize)),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::{KeyEvent, KeyEventState, MouseEvent};
    use termplate_core::frame::Role;

    #[test]
    fn colors_map_to_crossterm() {
        assert_eq!(to_crossterm(Color::Rgb(1, 2, 3)), CColor::Rgb { r: 1, g: 2, b: 3 });
        assert_eq!(to_crossterm(Color::DarkGrey), CColor::DarkGrey);
        assert_eq!(to_crossterm(Color::Reset), CColor::Reset);
    }

    #[test]
    fn no_color_env() {
        assert!(no_color_from_env(Some("1")));
        assert!(!no_color_from_env(Some("")));
        assert!(!no_color_from_env(None));
    }

    #[test]
    fn size_fallbacks() {
        assert_eq!(size_or_fallback(Some((120, 40)), Some("10"), Some("10")), (120, 40));
        assert_eq!(size_or_fallback(Some((0, 0)), Some("100"), Some("30")), (100, 30));
        assert_eq!(size_or_fallback(None, None, Some("x")), (80, 24));
    }

    fn text(bytes: &[u8]) -> String {
        String::from_utf8_lossy(bytes).into_owned()
    }

    #[test]
    fn first_draw_is_full_and_second_sends_only_changes() {
        let mut s = Screen::new(Vec::new(), StyleSheet::default(), false);
        let mut f = Frame::new(5, 2);
        f.text(0, 0, 5, "hello", Role::Normal);
        s.draw(&f).unwrap();
        let first = text(&s.out);
        assert!(first.contains("hello"));
        assert!(first.contains("\x1b[2J"), "clears the screen");
        s.out.clear();
        s.draw(&f).unwrap();
        assert!(!text(&s.out).contains('h'), "nothing changed");
        s.out.clear();
        f.put(4, 1, 'x', Role::Normal);
        s.draw(&f).unwrap();
        let third = text(&s.out);
        assert!(third.contains("\x1b[2;5H"), "moves to the changed cell: {third:?}");
        assert!(!third.contains("hello"));
    }

    #[test]
    fn invalidate_forces_full_redraw() {
        let mut s = Screen::new(Vec::new(), StyleSheet::default(), false);
        let mut f = Frame::new(3, 1);
        f.text(0, 0, 3, "abc", Role::Normal);
        s.draw(&f).unwrap();
        s.out.clear();
        s.invalidate();
        s.draw(&f).unwrap();
        assert!(text(&s.out).contains("abc"));
    }

    #[test]
    fn styles_come_from_role_and_area() {
        let mut sheet = StyleSheet::default();
        let row = sheet.parse_row(&["fg", "ff0000"]).unwrap();
        sheet.rows.insert("LOG".into(), row);
        let mut s = Screen::new(Vec::new(), sheet, false);
        let mut f = Frame::new(3, 1);
        f.put(0, 0, 'a', Role::Selected);
        f.set_area(vec!["LOG".into()]);
        f.put(1, 0, 'b', Role::Normal);
        s.draw(&f).unwrap();
        let out = text(&s.out);
        assert!(out.contains("\x1b[7m"), "reverse for selected: {out:?}");
        assert!(out.contains("\x1b[38;2;255;0;0m"), "area color: {out:?}");
    }

    #[test]
    fn no_color_drops_colors_keeps_attributes() {
        let mut s = Screen::new(Vec::new(), StyleSheet::default(), true);
        let mut f = Frame::new(2, 1);
        f.put(0, 0, 'a', Role::Stderr);
        f.put(1, 0, 'b', Role::Selected);
        s.draw(&f).unwrap();
        let out = text(&s.out);
        assert!(!out.contains("\x1b[38;"), "no foreground colors: {out:?}");
        assert!(out.contains("\x1b[7m"));
    }

    #[test]
    fn cell_styles_go_over_the_role_and_obey_no_color() {
        use termplate_core::style::Attrs;
        let red = Style { fg: Some(Color::Ansi(196)), attrs: Attrs { bold: true, ..Default::default() }, ..Default::default() };
        let mut f = Frame::new(3, 1);
        f.styled_text(0, 0, 3, "ab", Role::Normal, &[red, Style::default()]);
        let mut s = Screen::new(Vec::new(), StyleSheet::default(), false);
        s.draw(&f).unwrap();
        let out = text(&s.out);
        assert!(out.contains("\x1b[38;5;196m"), "{out:?}");
        assert!(out.contains("\x1b[1m"), "{out:?}");
        let mut s = Screen::new(Vec::new(), StyleSheet::default(), true);
        s.draw(&f).unwrap();
        let out = text(&s.out);
        assert!(!out.contains("\x1b[38;"), "NO_COLOR drops kept colors: {out:?}");
        assert!(out.contains("\x1b[1m"));
    }

    #[test]
    fn cursor_shown_for_inputs() {
        let mut s = Screen::new(Vec::new(), StyleSheet::default(), false);
        let mut f = Frame::new(4, 2);
        f.cursor = Some((2, 1));
        s.draw(&f).unwrap();
        assert!(text(&s.out).ends_with("\x1b[2;3H\x1b[?25h"));
    }

    fn key(code: CKey, mods: KeyModifiers) -> CEvent {
        CEvent::Key(KeyEvent { code, modifiers: mods, kind: KeyEventKind::Press, state: KeyEventState::NONE })
    }

    #[test]
    fn keys_map() {
        assert_eq!(map_event(key(CKey::Char('q'), KeyModifiers::CONTROL)), Some(InputEvent::Key(KeyCode::Char('q'), Mods { ctrl: true, ..Default::default() })));
        assert_eq!(map_event(key(CKey::Enter, KeyModifiers::NONE)), Some(InputEvent::key(KeyCode::Enter)));
        assert_eq!(map_event(key(CKey::CapsLock, KeyModifiers::NONE)), None);
        let release = CEvent::Key(KeyEvent { code: CKey::Enter, modifiers: KeyModifiers::NONE, kind: KeyEventKind::Release, state: KeyEventState::NONE });
        assert_eq!(map_event(release), None);
    }

    #[test]
    fn mouse_and_resize_map() {
        let m = |kind, mods| CEvent::Mouse(MouseEvent { kind, column: 3, row: 4, modifiers: mods });
        assert_eq!(map_event(m(MouseEventKind::Down(MouseButton::Left), KeyModifiers::NONE)), Some(InputEvent::click(3, 4)));
        assert_eq!(
            map_event(m(MouseEventKind::Down(MouseButton::Left), KeyModifiers::SHIFT)),
            Some(InputEvent::Mouse { binding: Binding::Mouse(Mouse::Click, Mods { shift: true, ..Default::default() }), x: 3, y: 4 })
        );
        assert_eq!(
            map_event(m(MouseEventKind::Down(MouseButton::Middle), KeyModifiers::NONE)),
            Some(InputEvent::Mouse { binding: Binding::mouse(Mouse::MiddleClick), x: 3, y: 4 })
        );
        assert_eq!(map_event(m(MouseEventKind::Moved, KeyModifiers::NONE)), Some(InputEvent::MouseMove { x: 3, y: 4 }));
        assert_eq!(map_event(CEvent::Resize(100, 30)), Some(InputEvent::Resize(100, 30)));
    }
}
