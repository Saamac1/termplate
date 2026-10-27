//! Key and mouse bindings and context stacking (RLML D 11.3, M 8; spec §9).
//!
//! A binding is written `ctrl+q`, `enter`, `k`, `?`, `f5`, `plus`, `num8`,
//! `click`, `shift+click`, `wheel.up`, or a gamepad button (`pad` blocks).
//! Lookup: `ALWAYS` first (it wins over every other context), then the
//! context stack from the top down. A `modal` context (the default) stops
//! the search; a `passthrough` context lets unbound input through.

use std::collections::BTreeMap;
use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Default)]
pub struct Mods {
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum KeyCode {
    Char(char),
    Enter,
    Esc,
    Tab,
    BackTab,
    Backspace,
    Delete,
    Insert,
    Up,
    Down,
    Left,
    Right,
    Home,
    End,
    PageUp,
    PageDown,
    F(u8),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Mouse {
    Click,
    RightClick,
    MiddleClick,
    WheelUp,
    WheelDown,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Binding {
    Key(KeyCode, Mods),
    Mouse(Mouse, Mods),
    Pad(String),
}

impl Binding {
    /// Normalizes a key event so that it compares equal to its parsed
    /// binding: shifted characters are written as such (`?`, `B`), ctrl/alt
    /// letters are lower case, and shift+tab is `BackTab`.
    pub fn key(code: KeyCode, mut mods: Mods) -> Binding {
        let code = match code {
            KeyCode::Char(c) if mods.ctrl || mods.alt => KeyCode::Char(c.to_ascii_lowercase()),
            KeyCode::Char(c) if mods.shift && c.is_ascii_alphabetic() => {
                mods.shift = false;
                KeyCode::Char(c.to_ascii_uppercase())
            }
            KeyCode::Tab if mods.shift => {
                mods.shift = false;
                KeyCode::BackTab
            }
            KeyCode::BackTab => {
                mods.shift = false;
                KeyCode::BackTab
            }
            other => other,
        };
        if let KeyCode::Char(c) = code
            && !c.is_ascii_alphabetic() && !mods.ctrl && !mods.alt {
                mods.shift = false;
            }
        Binding::Key(code, mods)
    }

    pub fn mouse(m: Mouse) -> Binding {
        Binding::Mouse(m, Mods::default())
    }
}

/// Gamepad buttons (D 11.3): position-based names.
pub const PAD_BUTTONS: &[&str] = &[
    "south", "east", "west", "north", "lb", "rb", "lt", "rt", "start", "select", "lstick", "rstick", "dpad.up",
    "dpad.down", "dpad.left", "dpad.right", "lstick.up", "lstick.down", "lstick.left", "lstick.right", "rstick.up",
    "rstick.down", "rstick.left", "rstick.right",
];

/// Parses a gamepad binding (`south`, `lb+start`).
pub fn parse_pad(s: &str) -> Result<Binding, String> {
    if s.split('+').all(|b| PAD_BUTTONS.contains(&b)) {
        Ok(Binding::Pad(s.to_string()))
    } else {
        Err(format!("unknown gamepad button `{s}`"))
    }
}

pub fn parse_binding(s: &str) -> Result<Binding, String> {
    let parts: Vec<&str> = if s == "+" { vec!["+"] } else { s.split('+').collect() };
    let (key, mod_parts) = parts.split_last().ok_or("empty key")?;
    let mut mods = Mods::default();
    for m in mod_parts {
        match *m {
            "ctrl" => mods.ctrl = true,
            "alt" => mods.alt = true,
            "shift" => mods.shift = true,
            other => return Err(format!("unknown modifier `{other}` in `{s}` (ctrl, alt, shift)")),
        }
    }
    let mouse = match *key {
        "click" => Some(Mouse::Click),
        "rclick" => Some(Mouse::RightClick),
        "mclick" => Some(Mouse::MiddleClick),
        "wheel.up" => Some(Mouse::WheelUp),
        "wheel.down" => Some(Mouse::WheelDown),
        _ => None,
    };
    if let Some(m) = mouse {
        return Ok(Binding::Mouse(m, mods));
    }
    let code = match *key {
        "enter" => KeyCode::Enter,
        "esc" => KeyCode::Esc,
        "tab" => KeyCode::Tab,
        "backspace" => KeyCode::Backspace,
        "delete" => KeyCode::Delete,
        "insert" => KeyCode::Insert,
        "up" => KeyCode::Up,
        "down" => KeyCode::Down,
        "left" => KeyCode::Left,
        "right" => KeyCode::Right,
        "home" => KeyCode::Home,
        "end" => KeyCode::End,
        "pageup" => KeyCode::PageUp,
        "pagedown" => KeyCode::PageDown,
        "space" => KeyCode::Char(' '),
        "plus" => KeyCode::Char('+'),
        "minus" => KeyCode::Char('-'),
        n if n.len() == 4 && n.starts_with("num") && n.as_bytes()[3].is_ascii_digit() => KeyCode::Char(n.as_bytes()[3] as char),
        f if f.len() >= 2 && f.starts_with('f') && f[1..].chars().all(|c| c.is_ascii_digit()) => {
            let n: u8 = f[1..].parse().map_err(|_| format!("bad function key `{s}`"))?;
            if !(1..=24).contains(&n) {
                return Err(format!("bad function key `{s}`"));
            }
            KeyCode::F(n)
        }
        "-" => return Err("write `-` as `minus`".into()),
        _ => {
            let mut chars = key.chars();
            match (chars.next(), chars.next()) {
                (Some(c), None) if !c.is_whitespace() => KeyCode::Char(c),
                _ => return Err(format!("unknown key `{s}`")),
            }
        }
    };
    Ok(Binding::key(code, mods))
}

impl fmt::Display for Binding {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mods = |f: &mut fmt::Formatter<'_>, m: &Mods| -> fmt::Result {
            if m.ctrl {
                f.write_str("Ctrl+")?;
            }
            if m.alt {
                f.write_str("Alt+")?;
            }
            if m.shift {
                f.write_str("Shift+")?;
            }
            Ok(())
        };
        match self {
            Binding::Pad(b) => write!(f, "{b}"),
            Binding::Mouse(m, md) => {
                mods(f, md)?;
                f.write_str(match m {
                    Mouse::Click => "Click",
                    Mouse::RightClick => "Right click",
                    Mouse::MiddleClick => "Middle click",
                    Mouse::WheelUp => "Wheel up",
                    Mouse::WheelDown => "Wheel down",
                })
            }
            Binding::Key(code, md) => {
                mods(f, md)?;
                match code {
                    KeyCode::Char(' ') => f.write_str("Space"),
                    KeyCode::Char(c) if md.ctrl || md.alt => write!(f, "{}", c.to_ascii_uppercase()),
                    KeyCode::Char(c) => write!(f, "{c}"),
                    KeyCode::F(n) => write!(f, "F{n}"),
                    KeyCode::PageUp => f.write_str("PgUp"),
                    KeyCode::PageDown => f.write_str("PgDn"),
                    KeyCode::BackTab => f.write_str("Shift+Tab"),
                    other => write!(f, "{other:?}"),
                }
            }
        }
    }
}

/// Built-in actions that may be bound to keys (spec §6, §9).
pub const KEY_ACTIONS: &[&str] = &[
    "quit", "back", "close", "confirm", "up", "down", "left", "right", "page_up", "page_down", "home", "end",
    "next_area", "prev_area", "refresh", "select",
];

/// RLML names accepted for Termplate's cursor actions.
pub fn action_alias(a: &str) -> &str {
    match a {
        "cursor.north" => "up",
        "cursor.south" => "down",
        "cursor.west" => "left",
        "cursor.east" => "right",
        other => other,
    }
}

/// The canonical (RLML) name of a cursor action, for display (spec §9).
pub fn canonical_action(a: &str) -> &str {
    match a {
        "up" => "cursor.north",
        "down" => "cursor.south",
        "left" => "cursor.west",
        "right" => "cursor.east",
        other => other,
    }
}

/// Engine contexts (RLML has more; Termplate uses these).
pub const BUILTIN_CONTEXTS: &[&str] = &["ALWAYS", "MENU", "INPUT", "LOG"];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Mode {
    #[default]
    Modal,
    Passthrough,
}

/// Default bindings (RLML M 8 plus Termplate's areas). An application's row
/// for the same action in the same context replaces the default.
pub const DEFAULT_KEYS: &[(&str, &str, &[&str])] = &[
    ("ALWAYS", "quit", &["ctrl+q"]),
    ("ALWAYS", "next_area", &["tab"]),
    ("ALWAYS", "prev_area", &["shift+tab"]),
    ("MENU", "up", &["up", "k"]),
    ("MENU", "down", &["down", "j"]),
    ("MENU", "confirm", &["enter", "space"]),
    ("MENU", "back", &["esc", "backspace"]),
    ("MENU", "left", &["left"]),
    ("MENU", "right", &["right"]),
    ("MENU", "page_up", &["pageup"]),
    ("MENU", "page_down", &["pagedown"]),
    ("MENU", "home", &["home"]),
    ("MENU", "end", &["end"]),
    ("MENU", "select", &["insert"]),
    ("INPUT", "confirm", &["enter"]),
    ("INPUT", "back", &["esc"]),
    ("LOG", "up", &["up", "k"]),
    ("LOG", "down", &["down", "j"]),
    ("LOG", "page_up", &["pageup"]),
    ("LOG", "page_down", &["pagedown"]),
    ("LOG", "home", &["home"]),
    ("LOG", "end", &["end"]),
    ("LOG", "back", &["esc"]),
];

/// One entry of the runtime context stack.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StackEntry {
    pub name: String,
    /// Forced passthrough (contexts of embedded layout areas).
    pub passthrough: bool,
}

impl StackEntry {
    pub fn modal(name: &str) -> Self {
        StackEntry { name: name.to_string(), passthrough: false }
    }
    pub fn passthrough(name: &str) -> Self {
        StackEntry { name: name.to_string(), passthrough: true }
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Keymap {
    /// context → ordered (action, bindings)
    contexts: BTreeMap<String, Vec<(String, Vec<Binding>)>>,
    modes: BTreeMap<String, Mode>,
}

impl Keymap {
    pub fn with_defaults() -> Self {
        let mut km = Keymap::default();
        for (ctx, action, keys) in DEFAULT_KEYS {
            let bindings = keys.iter().map(|k| parse_binding(k).expect("valid default key")).collect();
            km.contexts.entry(ctx.to_string()).or_default().push((action.to_string(), bindings));
        }
        km
    }

    pub fn set_mode(&mut self, context: &str, mode: Mode) {
        self.modes.insert(context.to_string(), mode);
    }

    pub fn mode(&self, context: &str) -> Mode {
        self.modes.get(context).copied().unwrap_or_default()
    }

    /// Default keys for `focus 1`…`focus 9` (spec §4): `alt+N` and `fN`
    /// in `ALWAYS`. Added after the application's keys: an action the app
    /// binds keeps its own keys, and a key the app uses anywhere is skipped.
    pub fn add_focus_defaults(&mut self) {
        for n in 1..=9 {
            let action = format!("focus {n}");
            if self.contexts.values().flatten().any(|(a, _)| *a == action) {
                continue;
            }
            let keys: Vec<Binding> = [format!("alt+{n}"), format!("f{n}")]
                .iter()
                .map(|k| parse_binding(k).expect("valid default key"))
                .filter(|b| !self.contexts.values().flatten().any(|(_, bs)| bs.contains(b)))
                .collect();
            if !keys.is_empty() {
                self.contexts.entry("ALWAYS".to_string()).or_default().push((action, keys));
            }
        }
    }

    /// Sets the bindings of `action` in `context`, replacing earlier ones.
    /// A key taken from another action in the same context is removed from
    /// that action (the explicit binding wins over a default).
    pub fn bind(&mut self, context: &str, action: &str, bindings: Vec<Binding>) {
        let list = self.contexts.entry(context.to_string()).or_default();
        for (_, bs) in list.iter_mut() {
            bs.retain(|b| !bindings.contains(b));
        }
        match list.iter_mut().find(|(a, _)| a == action) {
            Some((_, bs)) => *bs = bindings,
            None => list.push((action.to_string(), bindings)),
        }
    }

    fn find(&self, ctx: &str, input: &Binding) -> Option<&str> {
        let list = self.contexts.get(ctx)?;
        list.iter().find(|(_, bs)| bs.contains(input)).map(|(a, _)| a.as_str())
    }

    /// Finds the action for `input`; `stack` is ordered bottom first.
    pub fn lookup(&self, stack: &[StackEntry], input: &Binding) -> Option<&str> {
        if let Some(a) = self.find("ALWAYS", input) {
            return Some(a);
        }
        for e in stack.iter().rev().filter(|e| e.name != "ALWAYS") {
            if let Some(a) = self.find(&e.name, input) {
                return Some(a);
            }
            if !e.passthrough && self.mode(&e.name) == Mode::Modal {
                return None;
            }
        }
        None
    }

    /// The first binding of `action`, searched from the top of the stack
    /// down and then in `ALWAYS` (for `{^action}`, M 7.3).
    pub fn binding_for(&self, stack: &[StackEntry], action: &str) -> Option<&Binding> {
        let names = stack.iter().rev().map(|e| e.name.as_str()).filter(|n| *n != "ALWAYS").chain(std::iter::once("ALWAYS"));
        for ctx in names {
            if let Some((_, bs)) = self.contexts.get(ctx).and_then(|l| l.iter().find(|(a, _)| a == action))
                && let Some(b) = bs.first() {
                    return Some(b);
                }
        }
        None
    }

    /// The first binding of `action` in any context.
    pub fn any_binding_for(&self, action: &str) -> Option<&Binding> {
        let stack: Vec<StackEntry> = self.contexts.keys().map(|c| StackEntry::modal(c)).collect();
        self.binding_for(&stack, action)
    }

    /// Keys bound in `ALWAYS` that are also bound in another context: the
    /// other binding can never fire (RLML warns about this).
    pub fn always_shadowing(&self) -> Vec<(String, Binding)> {
        let Some(always) = self.contexts.get("ALWAYS") else { return vec![] };
        let mut out = Vec::new();
        for (ctx, list) in self.contexts.iter().filter(|(c, _)| *c != "ALWAYS") {
            for (_, bs) in list {
                for b in bs {
                    if always.iter().any(|(_, abs)| abs.contains(b)) {
                        out.push((ctx.clone(), b.clone()));
                    }
                }
            }
        }
        out
    }

    pub fn contexts(&self) -> impl Iterator<Item = (&String, &Vec<(String, Vec<Binding>)>)> {
        self.contexts.iter()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctrl() -> Mods {
        Mods { ctrl: true, ..Default::default() }
    }

    fn b(s: &str) -> Binding {
        parse_binding(s).unwrap()
    }

    #[test]
    fn parse_named_and_modified_keys() {
        assert_eq!(parse_binding("ctrl+q"), Ok(Binding::Key(KeyCode::Char('q'), ctrl())));
        assert_eq!(parse_binding("enter"), Ok(Binding::Key(KeyCode::Enter, Mods::default())));
        assert_eq!(parse_binding("space"), Ok(Binding::Key(KeyCode::Char(' '), Mods::default())));
        assert_eq!(parse_binding("f5"), Ok(Binding::Key(KeyCode::F(5), Mods::default())));
        assert_eq!(b("plus"), Binding::Key(KeyCode::Char('+'), Mods::default()));
        assert_eq!(b("minus"), Binding::Key(KeyCode::Char('-'), Mods::default()));
        assert_eq!(b("num8"), b("8"));
        assert_eq!(b("shift+tab"), Binding::Key(KeyCode::BackTab, Mods::default()));
    }

    #[test]
    fn names_are_lower_case_vocabulary() {
        assert!(parse_binding("Enter").is_err());
        assert!(parse_binding("Ctrl+q").is_err());
        assert!(parse_binding("-").unwrap_err().contains("minus"));
    }

    #[test]
    fn shifted_characters_normalize() {
        assert_eq!(b("shift+b"), b("B"));
        assert_ne!(b("b"), b("B"));
        assert_eq!(Binding::key(KeyCode::Char('B'), Mods { shift: true, ..Default::default() }), b("B"));
        assert_eq!(Binding::key(KeyCode::Char('?'), Mods { shift: true, ..Default::default() }), b("?"));
        assert_eq!(Binding::key(KeyCode::Char('#'), Mods { shift: true, ..Default::default() }), b("#"));
        assert_eq!(Binding::key(KeyCode::Tab, Mods { shift: true, ..Default::default() }), b("shift+tab"));
    }

    #[test]
    fn mouse_names_with_modifiers() {
        assert_eq!(b("click"), Binding::mouse(Mouse::Click));
        assert_eq!(b("mclick"), Binding::mouse(Mouse::MiddleClick));
        assert_eq!(b("wheel.down"), Binding::mouse(Mouse::WheelDown));
        assert_eq!(b("shift+click"), Binding::Mouse(Mouse::Click, Mods { shift: true, ..Default::default() }));
        assert!(parse_binding("wheel_up").is_err());
    }

    #[test]
    fn pad_buttons() {
        assert_eq!(parse_pad("south"), Ok(Binding::Pad("south".into())));
        assert_eq!(parse_pad("lb+start"), Ok(Binding::Pad("lb+start".into())));
        assert!(parse_pad("x").is_err());
    }

    #[test]
    fn invalid_keys() {
        assert!(parse_binding("hyper+x").is_err());
        assert!(parse_binding("foo").is_err());
        assert!(parse_binding("f99").is_err());
    }

    #[test]
    fn display_forms() {
        assert_eq!(b("ctrl+q").to_string(), "Ctrl+Q");
        assert_eq!(b("enter").to_string(), "Enter");
        assert_eq!(b("space").to_string(), "Space");
        assert_eq!(b("f2").to_string(), "F2");
        assert_eq!(b("shift+click").to_string(), "Shift+Click");
    }

    fn stack(names: &[&str]) -> Vec<StackEntry> {
        names.iter().map(|n| StackEntry::modal(n)).collect()
    }

    #[test]
    fn always_wins_over_other_contexts() {
        let mut km = Keymap::with_defaults();
        km.bind("MENU", "back", vec![b("ctrl+q")]);
        assert_eq!(km.lookup(&stack(&["MAIN", "MENU"]), &b("ctrl+q")), Some("quit"));
        assert_eq!(km.always_shadowing(), [("MENU".to_string(), b("ctrl+q"))]);
    }

    #[test]
    fn modal_context_blocks_lower_ones() {
        let mut km = Keymap::with_defaults();
        km.bind("MAIN", "BACKUP", vec![b("b")]);
        assert_eq!(km.lookup(&stack(&["MAIN"]), &b("b")), Some("BACKUP"));
        assert_eq!(km.lookup(&stack(&["MAIN", "MENU"]), &b("b")), None, "MENU is modal");
        // an embedded area's context lets unbound keys through
        let s = vec![StackEntry::modal("MAIN"), StackEntry::passthrough("MENU")];
        assert_eq!(km.lookup(&s, &b("b")), Some("BACKUP"));
        assert_eq!(km.lookup(&s, &b("j")), Some("down"));
    }

    #[test]
    fn passthrough_mode_from_keys_header() {
        let mut km = Keymap::with_defaults();
        km.bind("POPUP", "close", vec![b("backspace")]);
        assert_eq!(km.lookup(&stack(&["MENU", "POPUP"]), &b("j")), None);
        km.set_mode("POPUP", Mode::Passthrough);
        assert_eq!(km.lookup(&stack(&["MENU", "POPUP"]), &b("j")), Some("down"));
        assert_eq!(km.lookup(&stack(&["MENU", "POPUP"]), &b("backspace")), Some("close"), "POPUP's own binding first");
    }

    #[test]
    fn bind_replaces_action_and_steals_key() {
        let mut km = Keymap::with_defaults();
        km.bind("MENU", "confirm", vec![b("enter"), b("insert")]);
        assert_eq!(km.lookup(&stack(&["MENU"]), &b("insert")), Some("confirm"));
        assert_eq!(km.lookup(&stack(&["MENU"]), &b("space")), None);
        assert_eq!(km.binding_for(&stack(&["MENU"]), "select"), None);
    }

    #[test]
    fn binding_for_placeholder() {
        let km = Keymap::with_defaults();
        assert_eq!(km.binding_for(&stack(&["MENU"]), "confirm").unwrap().to_string(), "Enter");
        assert_eq!(km.binding_for(&stack(&["MENU"]), "quit").unwrap().to_string(), "Ctrl+Q");
        assert_eq!(km.any_binding_for("back").unwrap().to_string(), "Esc");
    }

    #[test]
    fn rlml_cursor_aliases() {
        assert_eq!(action_alias("cursor.north"), "up");
        assert_eq!(action_alias("confirm"), "confirm");
        assert_eq!(canonical_action("up"), "cursor.north");
        assert_eq!(canonical_action(action_alias("cursor.east")), "cursor.east");
        assert_eq!(canonical_action("confirm"), "confirm");
    }

    #[test]
    fn focus_defaults_skip_keys_the_app_uses() {
        let mut km = Keymap::with_defaults();
        km.bind("MAIN", "HELP", vec![parse_binding("f1").unwrap()]);
        km.bind("ALWAYS", "focus 3", vec![parse_binding("ctrl+t").unwrap()]);
        km.add_focus_defaults();
        let st = [StackEntry::modal("MAIN")];
        assert_eq!(km.lookup(&st, &parse_binding("f1").unwrap()), Some("HELP"));
        assert_eq!(km.lookup(&st, &parse_binding("alt+1").unwrap()), Some("focus 1"));
        assert_eq!(km.lookup(&st, &parse_binding("f2").unwrap()), Some("focus 2"));
        assert_eq!(km.lookup(&st, &parse_binding("alt+3").unwrap()), None, "app binding of focus 3 replaces defaults");
        assert!(km.always_shadowing().is_empty());
    }
}
