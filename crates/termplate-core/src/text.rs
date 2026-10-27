//! Texts and languages (spec §10).
//!
//! Lookup order: selected language → the app's default language → inline
//! default text (`- BACKUP "Varmuuskopioi"`) → the key itself.
//! Placeholders: `{NAME}` (variable/argument), `{@KEY}` (another text),
//! `{^action}` (current key binding), `{{` / `}}` (literal braces).

use std::collections::BTreeMap;

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Texts {
    pub langs: BTreeMap<String, BTreeMap<String, String>>,
    pub defaults: BTreeMap<String, String>,
    pub language: Option<String>,
    pub fallback_language: Option<String>,
}

const MAX_DEPTH: usize = 8;

impl Texts {
    pub fn insert(&mut self, lang: &str, key: &str, text: &str) {
        self.langs.entry(lang.to_string()).or_default().insert(key.to_string(), text.to_string());
    }

    /// Raw text for `key`, or `None` when no source has it.
    pub fn lookup(&self, key: &str) -> Option<&str> {
        for lang in [&self.language, &self.fallback_language].into_iter().flatten() {
            if let Some(t) = self.langs.get(lang).and_then(|m| m.get(key)) {
                return Some(t);
            }
        }
        self.defaults.get(key).map(String::as_str)
    }

    pub fn has(&self, key: &str) -> bool {
        self.lookup(key).is_some()
    }

    /// Resolved text; a missing text shows as its key.
    pub fn get(&self, key: &str, ctx: &dyn TextContext) -> String {
        match self.lookup(key) {
            Some(t) => self.format(t, ctx, 0),
            None => key.to_string(),
        }
    }

    /// Like [`Texts::get`] but returns `None` for a missing text.
    pub fn get_opt(&self, key: &str, ctx: &dyn TextContext) -> Option<String> {
        self.lookup(key).map(|t| self.format(t, ctx, 0))
    }

    pub fn format(&self, template: &str, ctx: &dyn TextContext, depth: usize) -> String {
        let mut out = String::new();
        let mut chars = template.chars().peekable();
        while let Some(c) = chars.next() {
            match c {
                '{' if chars.peek() == Some(&'{') => {
                    chars.next();
                    out.push('{');
                }
                '}' if chars.peek() == Some(&'}') => {
                    chars.next();
                    out.push('}');
                }
                '{' => {
                    let mut inner = String::new();
                    let mut closed = false;
                    for c in chars.by_ref() {
                        if c == '}' {
                            closed = true;
                            break;
                        }
                        inner.push(c);
                    }
                    if !closed {
                        out.push('{');
                        out.push_str(&inner);
                        continue;
                    }
                    out.push_str(&self.placeholder(&inner, ctx, depth));
                }
                c => out.push(c),
            }
        }
        out
    }

    fn placeholder(&self, inner: &str, ctx: &dyn TextContext, depth: usize) -> String {
        if let Some(key) = inner.strip_prefix('@') {
            if depth >= MAX_DEPTH {
                return format!("{{{inner}}}");
            }
            return match self.lookup(key) {
                Some(t) => self.format(t, ctx, depth + 1),
                None => key.to_string(),
            };
        }
        if let Some(action) = inner.strip_prefix('^') {
            return ctx.key_for(action).unwrap_or_else(|| format!("{{{inner}}}"));
        }
        ctx.var(inner).unwrap_or_else(|| format!("{{{inner}}}"))
    }
}

/// Supplies placeholder values while formatting.
pub trait TextContext {
    fn var(&self, name: &str) -> Option<String>;
    fn key_for(&self, action: &str) -> Option<String>;
}

/// A context with nothing in it.
pub struct NoContext;

impl TextContext for NoContext {
    fn var(&self, _: &str) -> Option<String> {
        None
    }
    fn key_for(&self, _: &str) -> Option<String> {
        None
    }
}

/// Picks the language code from a `LANG`-style value: `fi_FI.UTF-8` → `fi`.
/// `C` and `POSIX` give `None`.
pub fn language_from_env(lang: &str) -> Option<String> {
    let code = lang.split(['.', '@']).next()?.split(['_', '-']).next()?;
    if code.is_empty() || code == "C" || code == "POSIX" {
        None
    } else {
        Some(code.to_ascii_lowercase())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    struct Ctx(HashMap<&'static str, &'static str>);
    impl TextContext for Ctx {
        fn var(&self, name: &str) -> Option<String> {
            self.0.get(name).map(|s| s.to_string())
        }
        fn key_for(&self, action: &str) -> Option<String> {
            (action == "confirm").then(|| "Enter".to_string())
        }
    }

    fn texts() -> Texts {
        let mut t = Texts::default();
        t.insert("fi", "MAIN_MENU.BACKUP", "Varmuuskopioi");
        t.insert("en", "MAIN_MENU.BACKUP", "Back up");
        t.insert("en", "MAIN_MENU.QUIT", "Quit");
        t.insert("fi", "HELP", "Paina {^confirm} jatkaaksesi kohteeseen {TARGET}. {@MAIN_MENU.BACKUP}!");
        t.defaults.insert("MAIN_MENU.RESTORE".into(), "Palauta".into());
        t.language = Some("fi".into());
        t.fallback_language = Some("en".into());
        t
    }

    #[test]
    fn lookup_order() {
        let t = texts();
        assert_eq!(t.get("MAIN_MENU.BACKUP", &NoContext), "Varmuuskopioi");
        assert_eq!(t.get("MAIN_MENU.QUIT", &NoContext), "Quit", "falls back to default language");
        assert_eq!(t.get("MAIN_MENU.RESTORE", &NoContext), "Palauta", "inline default");
        assert_eq!(t.get("MAIN_MENU.NOPE", &NoContext), "MAIN_MENU.NOPE", "missing shows key");
        assert_eq!(t.get_opt("MAIN_MENU.NOPE", &NoContext), None);
    }

    #[test]
    fn language_beats_inline_default() {
        let mut t = texts();
        t.defaults.insert("MAIN_MENU.BACKUP".into(), "Inline".into());
        assert_eq!(t.get("MAIN_MENU.BACKUP", &NoContext), "Varmuuskopioi");
    }

    #[test]
    fn placeholders() {
        let t = texts();
        let ctx = Ctx(HashMap::from([("TARGET", "/mnt")]));
        assert_eq!(t.get("HELP", &ctx), "Paina Enter jatkaaksesi kohteeseen /mnt. Varmuuskopioi!");
    }

    #[test]
    fn unknown_placeholders_stay_visible() {
        let t = Texts::default();
        assert_eq!(t.format("{X} {^nope} {@MISSING}", &NoContext, 0), "{X} {^nope} MISSING");
    }

    #[test]
    fn brace_escapes_and_unclosed() {
        let t = Texts::default();
        assert_eq!(t.format("{{x}} {", &NoContext, 0), "{x} {");
    }

    #[test]
    fn reference_cycles_terminate() {
        let mut t = Texts::default();
        t.insert("fi", "A", "a{@B}");
        t.insert("fi", "B", "b{@A}");
        t.language = Some("fi".into());
        let s = t.get("A", &NoContext);
        assert!(s.starts_with("abab"));
        assert!(s.ends_with("{@A}") || s.ends_with("{@B}"));
    }

    #[test]
    fn env_language() {
        assert_eq!(language_from_env("fi_FI.UTF-8"), Some("fi".into()));
        assert_eq!(language_from_env("en-US"), Some("en".into()));
        assert_eq!(language_from_env("sv"), Some("sv".into()));
        assert_eq!(language_from_env("C.UTF-8"), None);
        assert_eq!(language_from_env("POSIX"), None);
        assert_eq!(language_from_env(""), None);
    }
}
