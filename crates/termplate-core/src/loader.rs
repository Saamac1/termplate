//! Source loading and block merging (RLML D 3, D 4.5, D 4.8; spec §2, §13).
//!
//! * `include` looks in the including file's folder, then in the
//!   application root. `*` matches one level and `**` any depth; wildcards
//!   take `.termplate`, `.rlml`, `.md` and `.rlml.md` files and skip names
//!   starting with `_` or `.` and `README` files.
//! * A file whose name is a language code (`text/fi.rlml`) is a language
//!   file: `KEY text` lines without a header (D 10).
//! * A later definition with the same full name replaces an earlier one; a
//!   replacement from another file warns unless marked `override`.
//!   Unnamed sections merge row by row. `patch` blocks apply after
//!   everything is loaded. User override files patch everything. A
//!   patch's `run:` script replaces the command's script, or its `- run`
//!   row, as a whole (spec §6.1).
//!
//! The core does no I/O; file access goes through [`SourceFs`].

use std::collections::{BTreeMap, HashSet};

use crate::error::{LoadError, LoadErrors};
use crate::names::{candidates, qualify};
use crate::syntax::{self, Directive, Entry, Modifier, RawBlock, RawItem, DRAWING_KINDS, SECTION_KINDS};

pub trait SourceFs {
    fn read(&self, path: &str) -> Result<String, String>;
    /// Paths matching `pattern`: `*` and `?` stay within one path segment,
    /// `**` matches any number of segments.
    fn glob(&self, pattern: &str) -> Result<Vec<String>, String>;
}

/// In-memory file system, for tests and embedding.
#[derive(Debug, Default, Clone)]
pub struct MemFs {
    pub files: BTreeMap<String, String>,
}

impl MemFs {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn with(mut self, path: &str, content: &str) -> Self {
        self.files.insert(normalize(path), content.to_string());
        self
    }
}

impl SourceFs for MemFs {
    fn read(&self, path: &str) -> Result<String, String> {
        self.files.get(&normalize(path)).cloned().ok_or_else(|| format!("file not found: {path}"))
    }
    fn glob(&self, pattern: &str) -> Result<Vec<String>, String> {
        let pattern = normalize(pattern);
        Ok(self.files.keys().filter(|p| glob_match(&pattern, p)).cloned().collect())
    }
}

/// Shell-style match: `*` and `?` do not cross `/`; `**` does.
pub fn glob_match(pattern: &str, path: &str) -> bool {
    let p: Vec<char> = pattern.chars().collect();
    let s: Vec<char> = path.chars().collect();
    fn rec(p: &[char], s: &[char]) -> bool {
        match p.first() {
            None => s.is_empty(),
            Some('*') if p.get(1) == Some(&'*') => {
                // `**/` may also match no folder at all
                let rest = if p.get(2) == Some(&'/') { &p[3..] } else { &p[2..] };
                (0..=s.len()).any(|i| rec(rest, &s[i..]))
            }
            Some('*') => {
                let mut i = 0;
                loop {
                    if rec(&p[1..], &s[i..]) {
                        return true;
                    }
                    if i >= s.len() || s[i] == '/' {
                        return false;
                    }
                    i += 1;
                }
            }
            Some('?') => !s.is_empty() && s[0] != '/' && rec(&p[1..], &s[1..]),
            Some(c) => !s.is_empty() && s[0] == *c && rec(&p[1..], &s[1..]),
        }
    }
    rec(&p, &s)
}

pub fn has_wildcard(pattern: &str) -> bool {
    pattern.contains(['*', '?'])
}

/// Resolves `.` and `..` segments and duplicate slashes.
pub fn normalize(path: &str) -> String {
    let absolute = path.starts_with('/');
    let mut parts: Vec<&str> = Vec::new();
    for seg in path.split('/') {
        match seg {
            "" | "." => {}
            ".." => {
                if matches!(parts.last(), Some(p) if *p != "..") {
                    parts.pop();
                } else if !absolute {
                    parts.push("..");
                }
            }
            s => parts.push(s),
        }
    }
    let joined = parts.join("/");
    if absolute { format!("/{joined}") } else { joined }
}

pub fn dirname(path: &str) -> &str {
    match path.rfind('/') {
        Some(0) => "/",
        Some(i) => &path[..i],
        None => "",
    }
}

pub fn join(dir: &str, rel: &str) -> String {
    if rel.starts_with('/') || dir.is_empty() {
        normalize(rel)
    } else {
        normalize(&format!("{dir}/{rel}"))
    }
}

/// Extensions taken by a wildcard (longest first).
pub const EXTENSIONS: &[&str] = &[".rlml.md", ".termplate", ".rlml", ".md"];

fn basename(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

/// The file name without a data-file extension.
pub fn stem(path: &str) -> &str {
    let base = basename(path);
    EXTENSIONS.iter().find_map(|e| base.strip_suffix(e)).unwrap_or(base)
}

/// Files a wildcard skips: `_x`, `.x`, `README.*`.
pub fn skipped_by_wildcard(path: &str) -> bool {
    let base = basename(path);
    base.starts_with('_') || base.starts_with('.') || stem(path) == "README"
}

/// `fi`, `en`, `pt_BR`, `zh-Hant`: a language file name (D 10).
pub fn language_of(path: &str) -> Option<&str> {
    if !EXTENSIONS.iter().any(|e| basename(path).ends_with(e)) {
        return None;
    }
    let s = stem(path);
    let (lang, region) = match s.split_once(['_', '-']) {
        Some((l, r)) => (l, Some(r)),
        None => (s, None),
    };
    let lang_ok = (2..=3).contains(&lang.len()) && lang.chars().all(|c| c.is_ascii_lowercase());
    let region_ok = region.is_none_or(|r| (2..=4).contains(&r.len()) && r.chars().all(|c| c.is_ascii_alphanumeric()));
    (lang_ok && region_ok).then_some(s)
}

/// A language file's first data line is a `KEY text` row (upper case); a
/// data file starts with a lower-case block kind or directive.
pub fn looks_like_language_file(src: &str) -> bool {
    src.lines()
        .map(str::trim)
        .find(|l| !l.is_empty() && !syntax::is_comment(l) && !l.starts_with('#') && !l.starts_with("namespace "))
        .is_none_or(|l| l.starts_with(|c: char| c.is_ascii_uppercase()))
}

struct Loader<'a> {
    fs: &'a dyn SourceFs,
    root_dir: String,
    stack: Vec<String>,
    seen: HashSet<String>,
    out: Vec<RawBlock>,
    errors: LoadErrors,
    warnings: Vec<LoadError>,
}

/// Loads `root` and everything it includes, depth first, in source order.
/// Returns the blocks and load warnings.
pub fn load_tree(fs: &dyn SourceFs, root: &str) -> Result<(Vec<RawBlock>, Vec<LoadError>), LoadErrors> {
    let root = normalize(root);
    let mut l = Loader {
        fs,
        root_dir: dirname(&root).to_string(),
        stack: vec![],
        seen: HashSet::new(),
        out: vec![],
        errors: LoadErrors::default(),
        warnings: vec![],
    };
    l.load_file(&root);
    let Loader { out, errors, warnings, .. } = l;
    errors.into_result((out, warnings))
}

impl Loader<'_> {
    fn load_file(&mut self, path: &str) {
        let src = match self.fs.read(path) {
            Ok(s) => s,
            Err(e) => {
                self.errors.push(LoadError::new(path, 0, e));
                return;
            }
        };
        self.stack.push(path.to_string());
        self.seen.insert(path.to_string());
        if let Some(lang) = language_of(path).filter(|_| looks_like_language_file(&src)) {
            match syntax::parse_language(path, &src) {
                Ok((namespace, items)) => self.out.push(RawBlock {
                    modifier: Modifier::None,
                    kind: "text".into(),
                    name: lang.to_string(),
                    params: None,
                    args: vec![],
                    args_text: String::new(),
                    file: path.to_string(),
                    line: 1,
                    namespace,
                    drawing: vec![],
                    drawing_line: 0,
                    items,
                    legend: vec![],
                    has_legend: false,
                    script: None,
                }),
                Err(es) => self.errors.0.extend(es),
            }
            self.stack.pop();
            return;
        }
        let parsed = match syntax::parse(path, &src) {
            Ok(p) => p,
            Err(es) => {
                self.errors.0.extend(es);
                self.stack.pop();
                return;
            }
        };
        for entry in parsed.entries {
            match entry {
                Entry::Block(block) => self.out.push(*block),
                Entry::Directive(Directive::Include { pattern, line }) => self.include(path, &pattern, line),
            }
        }
        self.stack.pop();
    }

    fn include(&mut self, from: &str, pattern: &str, line: usize) {
        let mut bases = vec![dirname(from).to_string()];
        if !bases.contains(&self.root_dir) {
            bases.push(self.root_dir.clone());
        }
        let wild = has_wildcard(pattern);
        let mut matches = Vec::new();
        for base in &bases {
            let full = join(base, pattern);
            let mut found = match self.fs.glob(&full) {
                Ok(m) => m,
                Err(e) => {
                    self.errors.push(LoadError::new(from, line, e));
                    return;
                }
            };
            if wild {
                found.retain(|m| !skipped_by_wildcard(m) && EXTENSIONS.iter().any(|e| m.ends_with(e)));
            }
            if !found.is_empty() {
                found.sort();
                matches = found;
                break;
            }
        }
        if matches.is_empty() {
            if wild {
                self.warnings.push(LoadError::new(from, line, format!("include {pattern} matches no files")));
            } else {
                self.errors.push(LoadError::new(from, line, format!("include: file not found: {pattern}")));
            }
            return;
        }
        for m in matches {
            let m = normalize(&m);
            if self.stack.contains(&m) {
                self.errors.push(LoadError::new(from, line, format!("include cycle: {m}")));
            } else if !self.seen.contains(&m) {
                self.load_file(&m);
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MergeMode {
    /// Application sources (D 4.5, D 4.8).
    Base,
    /// User override files: every block is a patch (D 3.2).
    UserOverride,
}

/// The patch identity of a row: its first word, or `open X` / `toggle X`
/// for key rows that bind an action with an argument. Text keys and color
/// names are qualified with the row's namespace.
pub fn row_key(kind: &str, item: &RawItem) -> String {
    let t = &item.tokens;
    if kind == "keys" && t.len() >= 2 && (t[0].is("open") || t[0].is("toggle")) {
        return format!("{} {}", t[0].text, t[1].text);
    }
    let key = item.key();
    if kind == "text" || (kind == "colors" && crate::names::is_name(key)) {
        return qualify(item.namespace.as_deref(), key);
    }
    key.to_string()
}

/// Kinds whose names share one namespace (D 2.1: different kinds with the
/// same full name collide).
pub const DEFINITION_KINDS: &[&str] = &["layout", "panel", "menu", "input", "command", "log"];

fn row_prefix(item: &RawItem) -> Option<&'static str> {
    let t = item.tokens.first()?;
    if item.tokens.len() < 2 || t.quoted {
        return None;
    }
    ["override", "new", "drop"].into_iter().find(|p| t.text == *p)
}

struct Def {
    block: RawBlock,
    overridden: bool,
    count: usize,
}

/// Merges application blocks (Base), or applies user overrides onto merged
/// blocks (UserOverride). Returns the blocks and the load warnings.
pub fn merge(base: Vec<RawBlock>, blocks: Vec<RawBlock>, mode: MergeMode) -> Result<(Vec<RawBlock>, Vec<LoadError>), LoadErrors> {
    let mut errors = LoadErrors::default();
    let mut warnings = Vec::new();
    let mut order: Vec<(String, String)> = Vec::new();
    let mut defs: BTreeMap<(String, String), Def> = BTreeMap::new();
    for b in base {
        let key = (b.kind.clone(), b.name.clone());
        order.push(key.clone());
        defs.insert(key, Def { block: b, overridden: false, count: 1 });
    }
    let mut patches: Vec<RawBlock> = Vec::new();

    for mut b in blocks {
        let unqualified = b.kind == "text" || SECTION_KINDS.contains(&b.kind.as_str()) || matches!(b.kind.as_str(), "keys" | "app");
        if !unqualified && b.modifier != Modifier::Patch {
            b.name = qualify(b.namespace.as_deref(), &b.name);
        }
        let section = SECTION_KINDS.contains(&b.kind.as_str()) || b.kind == "text";
        let modifier = match (b.modifier, mode) {
            (Modifier::None, MergeMode::UserOverride) => Modifier::Patch,
            (m, _) => m,
        };
        match modifier {
            Modifier::Drop => errors.push(b.err("`drop` is written at the start of a row (`- drop KEY`), not before a block")),
            Modifier::New if mode == MergeMode::Base => {
                errors.push(b.err("`new` is only allowed in `patch` blocks and user override files"))
            }
            Modifier::Patch => patches.push(b),
            _ if section => {
                let key = (b.kind.clone(), b.name.clone());
                match defs.get_mut(&key) {
                    Some(def) if modifier == Modifier::Override => {
                        b.modifier = Modifier::None;
                        def.block = b;
                        def.overridden = true;
                        def.count += 1;
                    }
                    Some(def) => merge_section_rows(&mut def.block, b, &mut errors, &mut warnings),
                    None => {
                        let mut start = RawBlock { items: vec![], modifier: Modifier::None, ..b.clone() };
                        merge_section_rows(&mut start, b, &mut errors, &mut warnings);
                        order.push(key.clone());
                        defs.insert(key, Def { block: start, overridden: false, count: 2 });
                    }
                }
            }
            _ => {
                let key = (b.kind.clone(), b.name.clone());
                if mode == MergeMode::UserOverride && modifier == Modifier::New && defs.contains_key(&key) {
                    errors.push(b.err(format!("`new {} {}`: it already exists", b.kind, b.name)));
                    continue;
                }
                let is_override = modifier == Modifier::Override;
                b.modifier = Modifier::None;
                match defs.get_mut(&key) {
                    None => {
                        order.push(key.clone());
                        defs.insert(key, Def { block: b, overridden: is_override, count: 1 });
                    }
                    Some(def) => {
                        def.count += 1;
                        if is_override && def.overridden {
                            errors.push(b.err(format!(
                                "two `override` definitions of {} {} (the other is at {}:{})",
                                b.kind, b.name, def.block.file, def.block.line
                            )));
                        } else if !def.overridden {
                            if !is_override && def.block.file != b.file {
                                warnings.push(b.err(format!(
                                    "{} {} replaces the definition at {}:{} (write `override` if intended)",
                                    b.kind, b.name, def.block.file, def.block.line
                                )));
                            }
                            def.block = b;
                            def.overridden = is_override;
                        }
                        // else: an override wins regardless of load order
                    }
                }
            }
        }
    }

    if mode == MergeMode::Base {
        for def in defs.values() {
            if def.overridden && def.count < 2 {
                errors.push(def.block.err(format!("`override {} {}`: nothing to replace", def.block.kind, def.block.name)));
            }
        }
    }

    // Different kinds with the same full name.
    let mut by_name: BTreeMap<String, Vec<&RawBlock>> = BTreeMap::new();
    for d in defs.values().filter(|d| DEFINITION_KINDS.contains(&d.block.kind.as_str())) {
        let name = d.block.name.split('.').take_while(|p| crate::names::is_name(p)).collect::<Vec<_>>().join(".");
        let list = by_name.entry(name).or_default();
        if !list.iter().any(|b| b.kind == d.block.kind) {
            list.push(&d.block);
        }
    }
    if let Some(vars) = defs.get(&("vars".to_string(), String::new())) {
        for item in &vars.block.items {
            if let Some(o) = by_name.get(item.key()).and_then(|l| l.first()) {
                errors.push(item.err(format!("variable {} collides with {} {} at {}:{}", item.key(), o.kind, o.name, o.file, o.line)));
            }
        }
    }
    for (name, list) in &by_name {
        if list.len() > 1 {
            let (a, b) = (list[0], list[1]);
            errors.push(b.err(format!("{} {name} collides with {} {name} at {}:{}", b.kind, a.kind, a.file, a.line)));
        }
    }

    // Patches, in load order, onto the winning definitions.
    for p in patches {
        let section = SECTION_KINDS.contains(&p.kind.as_str()) || p.kind == "text";
        let target = if section || matches!(p.kind.as_str(), "keys" | "app") {
            let key = (p.kind.clone(), p.name.clone());
            defs.contains_key(&key).then_some(key)
        } else {
            candidates(p.namespace.as_deref(), &p.name).into_iter().map(|n| (p.kind.clone(), n)).find(|k| defs.contains_key(k))
        };
        let Some(key) = target else {
            if mode == MergeMode::UserOverride && section {
                // The user's settings (and other sections) always exist.
                let key = (p.kind.clone(), p.name.clone());
                order.push(key.clone());
                let start = RawBlock { items: vec![], modifier: Modifier::None, ..p.clone() };
                let def = defs.entry(key).or_insert(Def { block: start, overridden: false, count: 1 });
                apply_patch(&mut def.block, p, true, &mut errors);
                continue;
            }
            errors.push(p.err(format!("cannot patch: no {} {}", p.kind, p.name)));
            continue;
        };
        if DRAWING_KINDS.contains(&p.kind.as_str()) {
            errors.push(p.err(format!("a {} is replaced as a whole (`override {} {}`), it cannot be patched", p.kind, p.kind, p.name)));
            continue;
        }
        let lenient = mode == MergeMode::UserOverride && p.kind == "settings";
        apply_patch(&mut defs.get_mut(&key).unwrap().block, p, lenient, &mut errors);
    }

    let out = order.into_iter().filter_map(|k| defs.remove(&k)).map(|d| d.block).collect();
    errors.into_result((out, warnings))
}

/// Section rows: the same key replaces (warning across files unless the
/// row says `override`), a new key appends.
fn merge_section_rows(target: &mut RawBlock, src: RawBlock, errors: &mut LoadErrors, warnings: &mut Vec<LoadError>) {
    for item in src.items {
        let (item, marked) = match row_prefix(&item) {
            Some("override") => (item.shift(1), true),
            Some(p) => {
                errors.push(item.err(format!("`{p}` rows are only allowed in `patch` blocks and user override files")));
                continue;
            }
            None => (item, false),
        };
        let key = row_key(&target.kind, &item);
        match target.items.iter().position(|t| row_key(&target.kind, t) == key) {
            Some(i) => {
                let old = &target.items[i];
                if !marked && old.file != item.file {
                    warnings.push(item.err(format!("{key} replaces the row at {}:{} (write `- override {key}` if intended)", old.file, old.line)));
                }
                target.items[i] = item;
            }
            None if marked => errors.push(item.err(format!("`override {key}`: nothing to replace"))),
            None => target.items.push(item),
        }
    }
}

/// `- KEY …` replaces, `- new KEY …` adds, `- drop KEY` removes (D 4.8).
/// `lenient` lets unknown keys in without `new` (user settings).
fn apply_patch(target: &mut RawBlock, patch: RawBlock, lenient: bool, errors: &mut LoadErrors) {
    if !patch.args.is_empty() {
        target.args = patch.args;
        target.args_text = patch.args_text;
    }
    let kind = target.kind.clone();
    // `run:` and `- run` are one field: either replaces the other.
    let mut items = patch.items;
    if patch.script.is_some() {
        target.items.retain(|t| t.key() != "run");
        target.script = patch.script;
    } else if target.script.is_some() && let Some(i) = items.iter().position(|t| t.key() == "run") {
        target.script = None;
        target.items.push(items.remove(i));
    }
    for (rows, patch_rows) in [(&mut target.items, items), (&mut target.legend, patch.legend)] {
        for item in patch_rows {
            let prefix = row_prefix(&item);
            let row = if prefix.is_some() { item.shift(1) } else { item.clone() };
            let key = row_key(&kind, &row);
            let pos = rows.iter().position(|t| row_key(&kind, t) == key);
            match (prefix, pos) {
                (Some("new"), None) => rows.push(row),
                (Some("new"), Some(_)) => errors.push(item.err(format!("`new {key}`: it already exists"))),
                (Some("drop"), Some(i)) => {
                    rows.remove(i);
                }
                (Some("drop"), None) => errors.push(item.err(format!("cannot drop `{key}`: no such row"))),
                (_, Some(i)) => rows[i] = row,
                (_, None) if lenient => rows.push(row),
                (_, None) => {
                    let existing: Vec<String> = rows.iter().map(|t| row_key(&kind, t)).collect();
                    errors.push(item.err(format!(
                        "cannot patch `{key}`: no such row (use `new` to add; existing: {})",
                        if existing.is_empty() { "none".to_string() } else { existing.join(", ") }
                    )));
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(blocks: &[RawBlock]) -> Vec<String> {
        blocks.iter().map(|b| format!("{} {}", b.kind, b.name)).collect()
    }

    fn tree(fs: &MemFs, root: &str) -> Vec<RawBlock> {
        load_tree(fs, root).unwrap_or_else(|e| panic!("{e}")).0
    }

    #[test]
    fn patch_replaces_run_script() {
        let base = "command C\n- output LOG\nrun:\n```\necho old\n```\n";
        let fs = MemFs::new().with("a.termplate", base);
        let (blocks, _) = load_tree(&fs, "a.termplate").unwrap();
        let patch = |src: &str| {
            let fs = MemFs::new().with("u.termplate", src);
            let (user, _) = load_tree(&fs, "u.termplate").unwrap();
            merge(blocks.clone(), user, MergeMode::UserOverride).unwrap().0.remove(0)
        };
        let b = patch("command C\nrun:\n```bash\necho new\n```\n");
        assert_eq!(b.script.as_ref().unwrap().lines, ["echo new"]);
        assert_eq!(b.script.as_ref().unwrap().file, "u.termplate");
        let b = patch("command C\n- run htop\n");
        assert!(b.script.is_none());
        assert_eq!(b.items.iter().map(|i| i.text.as_str()).collect::<Vec<_>>(), ["output LOG", "run htop"]);
        let b = patch("command C\n- output OTHER\n");
        assert!(b.script.is_some(), "a patch without `run:` keeps the script");
    }

    #[test]
    fn glob_matching() {
        assert!(glob_match("ui/*", "ui/main.termplate"));
        assert!(!glob_match("ui/*", "ui/sub/x.termplate"));
        assert!(glob_match("ui/**", "ui/sub/x.termplate"));
        assert!(glob_match("ui/**/*.rlml", "ui/a.rlml"));
        assert!(glob_match("ui/**/*.rlml", "ui/a/b/c.rlml"));
        assert!(glob_match("ui/*.rlml", "ui/a.rlml"));
        assert!(!glob_match("ui/*.rlml", "ui/a.termplate"));
        assert!(glob_match("a?c", "abc"));
        assert!(!glob_match("a?c", "a/c"));
    }

    #[test]
    fn path_helpers() {
        assert_eq!(normalize("a/./b//c/../d"), "a/b/d");
        assert_eq!(normalize("../x"), "../x");
        assert_eq!(dirname("dir/app.termplate"), "dir");
        assert_eq!(dirname("app.termplate"), "");
        assert_eq!(join("dir", "ui/*"), "dir/ui/*");
        assert_eq!(stem("text/fi.rlml.md"), "fi");
        assert_eq!(stem("ui/main.termplate"), "main");
    }

    #[test]
    fn wildcard_skips() {
        assert!(skipped_by_wildcard("ui/_draft.rlml"));
        assert!(skipped_by_wildcard("ui/.hidden.rlml"));
        assert!(skipped_by_wildcard("ui/README.md"));
        assert!(!skipped_by_wildcard("ui/main.rlml"));
    }

    #[test]
    fn language_file_names() {
        assert_eq!(language_of("text/fi.rlml"), Some("fi"));
        assert_eq!(language_of("text/pt_BR.termplate"), Some("pt_BR"));
        assert_eq!(language_of("text/en.md"), Some("en"));
        assert_eq!(language_of("ui/main.termplate"), None);
        assert_eq!(language_of("fi"), None, "needs a data extension");
        assert_eq!(language_of("text/FI.rlml"), None);
    }

    #[test]
    fn includes_expand_in_place_and_sorted() {
        let fs = MemFs::new()
            .with("app/app.termplate", "app A\n- termplate 0.1\n\ninclude ui/*\nmenu LAST\n- X\n")
            .with("app/ui/b.termplate", "menu B\n- X\n")
            .with("app/ui/a.rlml.md", "# Menus\nmenu A\n- X\n")
            .with("app/ui/_draft.termplate", "menu DRAFT\n- X\n")
            .with("app/ui/README.md", "free text")
            .with("app/ui/notes.txt", "garbage");
        assert_eq!(names(&tree(&fs, "app/app.termplate")), ["app A", "menu A", "menu B", "menu LAST"]);
    }

    #[test]
    fn double_star_includes_subfolders() {
        let fs = MemFs::new()
            .with("a.termplate", "include ui/**\n")
            .with("ui/x.termplate", "menu X\n- A\n")
            .with("ui/sub/y.termplate", "menu Y\n- A\n");
        assert_eq!(names(&tree(&fs, "a.termplate")), ["menu Y", "menu X"]);
    }

    #[test]
    fn include_falls_back_to_app_root() {
        let fs = MemFs::new()
            .with("app/main.termplate", "include ui/menus.termplate\n")
            .with("app/ui/menus.termplate", "include lib/common.termplate\n")
            .with("app/lib/common.termplate", "menu COMMON\n- A\n");
        assert_eq!(names(&tree(&fs, "app/main.termplate")), ["menu COMMON"]);
    }

    #[test]
    fn language_detection_looks_at_content() {
        assert!(looks_like_language_file("> c\nPAUSE.TITLE Tauko\n"));
        assert!(!looks_like_language_file("app A\n- termplate 0.1\n"));
        assert!(!looks_like_language_file("# UI\ninclude x\n"));
        let fs = MemFs::new().with("app.termplate", "app A\n- termplate 0.1\n\ninclude ui.termplate\n").with("ui.termplate", "menu M\n- QUIT\n");
        assert_eq!(names(&tree(&fs, "app.termplate")), ["app A", "menu M"], "`app` and `ui` are not languages");
    }

    #[test]
    fn language_files_become_text_blocks() {
        let fs = MemFs::new().with("a.termplate", "include text/*\n").with("text/fi.rlml", "namespace UI\nPAUSE.TITLE Tauko\n");
        let b = &tree(&fs, "a.termplate")[0];
        assert_eq!((b.kind.as_str(), b.name.as_str()), ("text", "fi"));
        assert_eq!(b.namespace.as_deref(), Some("UI"));
        assert_eq!(b.items[0].rest(1), "Tauko");
    }

    #[test]
    fn wildcard_matching_nothing_warns_missing_file_errors() {
        let fs = MemFs::new().with("a.termplate", "include none/*\n");
        let (_, warnings) = load_tree(&fs, "a.termplate").unwrap();
        assert!(warnings[0].message.contains("matches no files"));
        let fs = MemFs::new().with("a.termplate", "include missing.termplate\n");
        assert!(load_tree(&fs, "a.termplate").unwrap_err().0[0].message.contains("missing.termplate"));
    }

    #[test]
    fn include_cycle_detected_and_diamond_loads_once() {
        let fs = MemFs::new().with("a.termplate", "include b.termplate\n").with("b.termplate", "include a.termplate\n");
        assert!(load_tree(&fs, "a.termplate").unwrap_err().0[0].message.contains("cycle"));
        let fs = MemFs::new()
            .with("a.termplate", "include b.termplate\ninclude c.termplate\n")
            .with("b.termplate", "include c.termplate\n")
            .with("c.termplate", "menu C\n- X\n");
        assert_eq!(names(&tree(&fs, "a.termplate")), ["menu C"]);
    }

    #[test]
    fn parse_errors_from_included_files_are_collected() {
        let fs = MemFs::new()
            .with("a.termplate", "include b.termplate\ninclude c.termplate\n")
            .with("b.termplate", "bogus Y\n")
            .with("c.termplate", "menu\n");
        assert_eq!(load_tree(&fs, "a.termplate").unwrap_err().0.len(), 2);
    }

    fn parse_in(file: &str, src: &str) -> Vec<RawBlock> {
        syntax::parse(file, src).unwrap().blocks().cloned().collect()
    }

    fn parse(src: &str) -> Vec<RawBlock> {
        parse_in("t", src)
    }

    fn base(blocks: Vec<RawBlock>) -> (Vec<RawBlock>, Vec<LoadError>) {
        merge(vec![], blocks, MergeMode::Base).unwrap_or_else(|e| panic!("{e}"))
    }

    fn base_err(blocks: Vec<RawBlock>) -> String {
        merge(vec![], blocks, MergeMode::Base).unwrap_err().to_string()
    }

    #[test]
    fn same_file_replacement_is_silent_other_file_warns() {
        let (out, w) = base(parse("menu A\n- X\n\nmenu A\n- Y\n"));
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].items[0].key(), "Y");
        assert!(w.is_empty());
        let mut blocks = parse_in("a", "menu A\n- X\n");
        blocks.extend(parse_in("b", "menu A\n- Y\n"));
        let (out, w) = base(blocks);
        assert_eq!(out[0].items[0].key(), "Y");
        assert!(w[0].message.contains("replaces the definition at a:1"), "{w:?}");
    }

    #[test]
    fn override_rules() {
        let mut blocks = parse_in("lib", "override menu A\n- O\n");
        blocks.extend(parse_in("app", "menu A\n- X\n"));
        let (out, w) = base(blocks);
        assert_eq!(out[0].items[0].key(), "O", "override wins regardless of order");
        assert!(w.is_empty());
        assert!(base_err(parse("override menu A\n- X\n")).contains("nothing to replace"));
        assert!(base_err(parse("menu A\n- X\n\noverride menu A\n- Y\n\noverride menu A\n- Z\n")).contains("two `override`"));
    }

    #[test]
    fn different_kinds_with_same_name_collide() {
        assert!(base_err(parse("menu A\n- X\n\npanel A\nx\n\nlegend:\n- x value 1\n")).contains("collides with"));
        assert!(base_err(parse("vars\n- A text\n\nmenu A\n- X\n")).contains("variable A collides with menu A"));
    }

    #[test]
    fn layout_variants_do_not_collide_with_base() {
        let (out, _) = base(parse("layout MAIN\na\n\nlegend:\n- a X\n\nlayout MAIN.narrow\na\n\nlegend:\n- a X\n"));
        assert_eq!(out.len(), 2);
    }

    #[test]
    fn sections_merge_rows() {
        let mut blocks = parse_in("a", "vars\n- A text\n- B text\n");
        blocks.extend(parse_in("b", "vars\n- C text\n- B number\n- override A bool\n"));
        let (out, w) = base(blocks);
        let rows: Vec<&str> = out[0].items.iter().map(|i| i.text.as_str()).collect();
        assert_eq!(rows, ["A bool", "B number", "C text"]);
        assert_eq!(w.len(), 1, "unmarked B replacement warns: {w:?}");
        assert!(base_err(parse("vars\n- override Z text\n")).contains("nothing to replace"));
        assert!(base_err(parse("vars\n- new Z text\n")).contains("only allowed in `patch`"));
    }

    #[test]
    fn text_keys_merge_per_language() {
        let mut blocks = parse_in("a", "text fi\nA a\n");
        blocks.extend(parse_in("b", "text fi\nB b\n"));
        let (out, _) = base(blocks);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].items.len(), 2);
    }

    #[test]
    fn patch_replaces_adds_and_drops_rows_after_load() {
        let mut blocks = parse("patch keys MENU\n- confirm enter space\n- new pause p\n- drop down\n");
        blocks.extend(parse("keys MENU\n- confirm enter\n- up k up\n- down j down\n"));
        let (out, _) = base(blocks);
        let items: Vec<&str> = out[0].items.iter().map(|i| i.text.as_str()).collect();
        assert_eq!(items, ["confirm enter space", "up k up", "pause p"]);
        assert_eq!(out[0].items[2].span(0, 2), "pause p");
    }

    #[test]
    fn patch_errors() {
        let b = parse("keys MENU\n- confirm enter\n");
        let with = |p: &str| {
            let mut v = b.clone();
            v.extend(parse(p));
            base_err(v)
        };
        assert!(with("patch keys MENU\n- confirn enter\n").contains("cannot patch `confirn`: no such row (use `new` to add; existing: confirm)"));
        assert!(with("patch keys MENU\n- new confirm x\n").contains("already exists"));
        assert!(with("patch keys MENU\n- drop up\n").contains("cannot drop `up`"));
        assert!(with("patch keys MENUU\n- confirm x\n").contains("no keys MENUU"));
        assert!(with("panel P\nx\n\nlegend:\n- x value 1\n\npatch panel P\n- x value 2\n").contains("cannot be patched"));
        assert!(with("new menu N\n- A\n").contains("only allowed in `patch`"));
        assert!(with("drop menu N\n").contains("start of a row"));
    }

    #[test]
    fn key_rows_with_open_and_toggle() {
        let mut v = parse("keys ALWAYS\n- open PAUSE esc\n- open HELP f1\n- toggle DEBUG f12\n");
        v.extend(parse("patch keys ALWAYS\n- open HELP f2\n"));
        let (out, _) = base(v);
        let rows: Vec<&str> = out[0].items.iter().map(|i| i.text.as_str()).collect();
        assert_eq!(rows, ["open PAUSE esc", "open HELP f2", "toggle DEBUG f12"]);
    }

    #[test]
    fn namespaces_prefix_definitions_and_patch_targets() {
        let mut v = parse("namespace UNDEAD\nmenu LOOT\n- X\n");
        v.extend(parse("menu LOOT\n- Y\n"));
        v.extend(parse("namespace UNDEAD\npatch menu LOOT\n- new Z\n"));
        let (out, _) = base(v);
        assert_eq!(names(&out), ["menu UNDEAD.LOOT", "menu LOOT"]);
        assert_eq!(out[0].items.len(), 2, "patch found UNDEAD.LOOT first");
    }

    #[test]
    fn user_overrides_patch_everything() {
        let (app, _) = base(parse("vars\n- TARGET text \"/mnt/backup\"\n- LEVEL number 3\n\nkeys GAME\n- QUAFF q\n"));
        let (out, _) = merge(app.clone(), parse("vars\n- TARGET text \"/srv/backup\"\n\nkeys GAME\n- QUAFF d\n- new LOOK l\n"), MergeMode::UserOverride).unwrap();
        assert_eq!(out[0].items[0].tokens[2].text, "/srv/backup");
        assert_eq!(out[0].items.len(), 2);
        let keys: Vec<&str> = out[1].items.iter().map(|i| i.text.as_str()).collect();
        assert_eq!(keys, ["QUAFF d", "LOOK l"]);
        assert!(merge(app.clone(), parse("keys GAEM\n- QUAFF d\n"), MergeMode::UserOverride).is_err(), "typo");
        assert!(merge(app.clone(), parse("keys GAME\n- LOOK l\n"), MergeMode::UserOverride).is_err(), "new row needs `new`");
        let (out, _) = merge(app.clone(), parse("new keys MACROS\n- QUAFF m\n"), MergeMode::UserOverride).unwrap();
        assert_eq!(out.last().unwrap().name, "MACROS");
        let (out, _) = merge(app, parse("override keys GAME\n- LOOK l\n"), MergeMode::UserOverride).unwrap();
        assert_eq!(out[1].items.len(), 1);
    }

    #[test]
    fn user_settings_need_no_new() {
        let (out, _) = merge(vec![], parse("settings\n- language en\n- mouse off\n"), MergeMode::UserOverride).unwrap();
        assert_eq!(out[0].kind, "settings");
        assert_eq!(out[0].items.len(), 2);
    }
}
