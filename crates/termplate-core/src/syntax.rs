//! Block-level syntax: the RLML base syntax (RLML D 2, D 4.2, D 4.8; spec §2).
//!
//! ```text
//! namespace TOOLS              (file start only)
//! include ui/*
//!
//! [patch|override|new] KIND NAME[(params)] [header words…]
//! <drawing>                    (layout and panel; may follow a blank line,
//!                               may be wrapped in a ``` fence)
//! - [override|new|drop] item words…
//! legend:                      (continues over blank lines until a header)
//! - legend item words…
//! run:                         (command only: then a code block, fenced
//!                               with backticks, holding the script; §6.1)
//! ```
//!
//! * A block ends at a blank line (D 2.2). Free text outside blocks is a
//!   load error; markdown headings (`#`) are skipped.
//! * Comments: a line whose first character is `>` followed by a space or
//!   the line end, and a trailing `**…**`, `***…***` or `~~…~~` (D 2.5).
//!   Drawings have no comments.
//! * A fenced code block is the drawing of the preceding drawing block;
//!   inside it every line, also blank ones, is drawing.
//! * `run:` in a command block takes the fenced code block after it as the
//!   command's script; the fence is required and its content is verbatim.
//! * `text LANG` blocks (Termplate) and header-less language files take
//!   bare `KEY text` lines (D 10).

use crate::error::LoadError;
use crate::token::{tokenize, Token};

pub const BLOCK_KINDS: &[&str] = &[
    "app", "vars", "layout", "panel", "command", "menu", "input", "keys", "text", "colors", "panels", "logs",
    "log", "settings",
];

pub const DRAWING_KINDS: &[&str] = &["layout", "panel"];

/// Unnamed sections: rows from every file merge by row key (D 4.8).
pub const SECTION_KINDS: &[&str] = &["vars", "colors", "settings", "panels", "logs"];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Modifier {
    None,
    Patch,
    Override,
    New,
    Drop,
}

#[derive(Debug, Clone, PartialEq)]
pub struct RawItem {
    pub file: String,
    pub line: usize,
    /// Namespace of the file the item was written in.
    pub namespace: Option<String>,
    /// Text after the leading `- `.
    pub text: String,
    pub tokens: Vec<Token>,
}

impl RawItem {
    pub fn new(file: &str, line: usize, namespace: Option<String>, text: &str) -> Result<RawItem, LoadError> {
        let tokens = tokenize(text).map_err(|e| LoadError::new(file, line, e.0))?;
        Ok(RawItem { file: file.to_string(), line, namespace, text: text.to_string(), tokens })
    }

    /// The first word.
    pub fn key(&self) -> &str {
        self.tokens.first().map(|t| t.text.as_str()).unwrap_or("")
    }

    pub fn err(&self, msg: impl Into<String>) -> LoadError {
        LoadError::new(&self.file, self.line, msg)
    }

    /// Original text of tokens `from..to`.
    pub fn span(&self, from: usize, to: usize) -> &str {
        crate::token::span_text(&self.text, &self.tokens, from, to)
    }

    /// Original text from token `from` to the end of the line.
    pub fn rest(&self, from: usize) -> &str {
        self.span(from, self.tokens.len())
    }

    /// The item without its first `n` words.
    pub fn shift(&self, n: usize) -> RawItem {
        let Some(first) = self.tokens.get(n) else {
            return RawItem { text: String::new(), tokens: vec![], ..self.clone() };
        };
        let off = first.start;
        RawItem {
            text: self.text[off..].to_string(),
            tokens: self.tokens[n..].iter().map(|t| Token { start: t.start - off, end: t.end - off, ..t.clone() }).collect(),
            ..self.clone()
        }
    }
}

/// A header parameter: `name` or `name: type`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Param {
    pub name: String,
    pub ty: Option<String>,
}

/// A command's `run:` script (spec §6.1).
#[derive(Debug, Clone, PartialEq)]
pub struct RawScript {
    /// The fence's language tag (`bash`), empty when none.
    pub lang: String,
    pub lines: Vec<String>,
    pub file: String,
    /// Line number of the first script line.
    pub line: usize,
}

impl RawScript {
    pub fn text(&self) -> String {
        let mut t = self.lines.join("\n");
        t.push('\n');
        t
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct RawBlock {
    pub modifier: Modifier,
    pub kind: String,
    pub name: String,
    /// `Some` when the header had `(…)` after the name.
    pub params: Option<Vec<Param>>,
    pub args: Vec<Token>,
    /// Header text after the name (for header expressions such as `when`).
    pub args_text: String,
    pub file: String,
    pub line: usize,
    pub namespace: Option<String>,
    pub drawing: Vec<String>,
    /// Line number of the first drawing line.
    pub drawing_line: usize,
    pub items: Vec<RawItem>,
    pub legend: Vec<RawItem>,
    pub has_legend: bool,
    pub script: Option<RawScript>,
}

impl RawBlock {
    pub fn err(&self, msg: impl Into<String>) -> LoadError {
        LoadError::new(&self.file, self.line, msg)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Directive {
    Include { pattern: String, line: usize },
}

/// Either a directive or a block, in file order.
#[derive(Debug, Clone, PartialEq)]
pub enum Entry {
    Directive(Directive),
    Block(Box<RawBlock>),
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct ParsedFile {
    pub file: String,
    pub namespace: Option<String>,
    pub entries: Vec<Entry>,
}

impl ParsedFile {
    pub fn blocks(&self) -> impl Iterator<Item = &RawBlock> {
        self.entries.iter().filter_map(|e| match e {
            Entry::Block(b) => Some(b.as_ref()),
            _ => None,
        })
    }
}

#[derive(Debug, PartialEq, Clone, Copy)]
enum State {
    /// Between blocks.
    Outside,
    /// In a block body.
    Body,
    /// A drawing block whose header was followed by a blank line: the
    /// drawing may still come.
    AwaitDrawing,
    /// Unfenced drawing: ends at a blank line or `legend:`.
    Drawing,
    /// Fenced drawing: ends at the closing fence.
    FencedDrawing,
    /// The block ended at a blank line; only `legend:` may continue it.
    Closed,
    Legend,
    /// Skipping a code block that is not a drawing.
    SkipFence,
    /// After `run:`: the script's opening fence must come.
    AwaitScript,
    /// Inside a script; ends at a fence at least as long as the opening one.
    Script(usize),
}

fn header_kind(line: &str) -> Option<(Modifier, &str)> {
    if line.starts_with(char::is_whitespace) {
        return None;
    }
    let mut words = line.split_whitespace();
    let first = words.next()?;
    let modifier = match first {
        "patch" => Modifier::Patch,
        "override" => Modifier::Override,
        "new" => Modifier::New,
        "drop" => Modifier::Drop,
        _ => Modifier::None,
    };
    let kind = if modifier == Modifier::None { first } else { words.next()? };
    BLOCK_KINDS.iter().find(|k| **k == kind).map(|k| (modifier, *k))
}

/// `>` followed by a space or the end of the line (D 2.5).
pub fn is_comment(trimmed: &str) -> bool {
    trimmed == ">" || trimmed.starts_with("> ")
}

/// Removes a trailing emphasis comment: `**…**`, `***…***` or `~~…~~` at
/// the end of the line, opened right before text and closed right after it.
pub fn strip_emphasis(line: &str) -> &str {
    let t = line.trim_end();
    for marker in ["***", "**", "~~"] {
        if !t.ends_with(marker) || t.len() < 2 * marker.len() + 1 {
            continue;
        }
        let body_end = t.len() - marker.len();
        if t[..body_end].ends_with(char::is_whitespace) {
            continue;
        }
        // earliest opening marker that starts a word
        let mut search = 0;
        while let Some(pos) = t[search..body_end].find(marker) {
            let start = search + pos;
            let at_word_start = start == 0 || t[..start].ends_with(char::is_whitespace);
            let after = &t[start + marker.len()..body_end];
            if at_word_start && !after.is_empty() && !after.starts_with(char::is_whitespace) && !after.starts_with('*') {
                return t[..start].trim_end();
            }
            search = start + marker.len();
        }
    }
    line
}

pub fn parse(file: &str, src: &str) -> Result<ParsedFile, Vec<LoadError>> {
    let mut out = ParsedFile { file: file.to_string(), namespace: None, entries: Vec::new() };
    let mut errors = Vec::new();
    let mut current: Option<RawBlock> = None;
    let mut state = State::Outside;
    let mut seen_definition = false;

    let flush = |current: &mut Option<RawBlock>, out: &mut ParsedFile| {
        if let Some(b) = current.take() {
            out.entries.push(Entry::Block(Box::new(b)));
        }
    };

    for (idx, raw_line) in src.lines().enumerate() {
        let lineno = idx + 1;
        let line = raw_line.trim_end_matches('\r');
        let trimmed = line.trim();
        let fence = line.trim_start().starts_with("```");

        match state {
            State::SkipFence => {
                if fence {
                    state = State::Outside;
                }
                continue;
            }
            State::Script(ticks) => {
                let t = line.trim();
                if t.len() >= ticks && t.chars().all(|c| c == '`') {
                    state = State::Body;
                } else {
                    current.as_mut().unwrap().script.as_mut().unwrap().lines.push(line.to_string());
                }
                continue;
            }
            State::AwaitScript => {
                if trimmed.is_empty() || is_comment(trimmed) {
                    continue;
                }
                if fence {
                    let t = line.trim_start();
                    let ticks = t.chars().take_while(|c| *c == '`').count();
                    let lang = t[ticks..].trim().to_string();
                    current.as_mut().unwrap().script = Some(RawScript { lang, lines: vec![], file: file.to_string(), line: lineno + 1 });
                    state = State::Script(ticks);
                } else {
                    errors.push(LoadError::new(file, lineno, "`run:` must be followed by a ``` code block holding the script"));
                    state = State::Closed;
                }
                continue;
            }
            State::FencedDrawing => {
                if fence {
                    state = State::Body;
                } else {
                    current.as_mut().unwrap().drawing.push(line.trim_end().to_string());
                }
                continue;
            }
            State::Drawing => {
                if trimmed.is_empty() || fence {
                    state = State::Closed;
                    continue;
                }
                if trimmed == "legend:" {
                    state = State::Legend;
                    current.as_mut().unwrap().has_legend = true;
                    continue;
                }
                current.as_mut().unwrap().drawing.push(line.trim_end().to_string());
                continue;
            }
            _ => {}
        }

        if fence {
            let lang = line.trim_start().trim_start_matches('`').trim();
            let drawing_pending = matches!(state, State::Body | State::AwaitDrawing)
                && current.as_ref().is_some_and(|b| DRAWING_KINDS.contains(&b.kind.as_str()) && b.drawing.is_empty());
            if lang.is_empty() && drawing_pending {
                let b = current.as_mut().unwrap();
                b.drawing_line = lineno + 1;
                state = State::FencedDrawing;
            } else {
                errors.push(LoadError::new(
                    file,
                    lineno,
                    if lang.is_empty() {
                        "a code block must hold the drawing of the layout or panel above it".to_string()
                    } else {
                        format!("`{lang}` code blocks are not supported by Termplate")
                    },
                ));
                flush(&mut current, &mut out);
                state = State::SkipFence;
            }
            continue;
        }

        if trimmed.is_empty() {
            state = match state {
                State::Legend => State::Legend,
                State::Body
                    if current.as_ref().is_some_and(|b| DRAWING_KINDS.contains(&b.kind.as_str()) && b.drawing.is_empty() && b.items.is_empty()) =>
                {
                    State::AwaitDrawing
                }
                State::AwaitDrawing => State::AwaitDrawing,
                State::Outside => State::Outside,
                _ => State::Closed,
            };
            continue;
        }
        if is_comment(trimmed) {
            continue;
        }

        // A drawing may start right after the header or after a blank line;
        // its first line can be anything but an item or `legend:`.
        if matches!(state, State::Body | State::AwaitDrawing)
            && !trimmed.starts_with("- ")
            && trimmed != "-"
            && trimmed != "legend:"
            && header_kind(line).is_none()
            && current.as_ref().is_some_and(|b| DRAWING_KINDS.contains(&b.kind.as_str()) && b.drawing.is_empty() && b.items.is_empty())
        {
            let b = current.as_mut().unwrap();
            b.drawing_line = lineno;
            b.drawing.push(line.trim_end().to_string());
            state = State::Drawing;
            continue;
        }

        if trimmed.starts_with('#') {
            // markdown heading
            continue;
        }

        let code = strip_emphasis(line);
        let code_trimmed = code.trim();

        if let Some((modifier, kind)) = header_kind(code) {
            flush(&mut current, &mut out);
            seen_definition = true;
            match parse_header(file, lineno, code, modifier, kind, &out.namespace) {
                Ok(b) => {
                    current = Some(b);
                    state = State::Body;
                }
                Err(e) => {
                    errors.push(e);
                    state = State::Closed;
                }
            }
            continue;
        }

        if !line.starts_with(char::is_whitespace) {
            let mut words = code_trimmed.split_whitespace();
            match words.next() {
                Some("include") => {
                    flush(&mut current, &mut out);
                    state = State::Outside;
                    let pattern: Vec<&str> = words.collect();
                    if pattern.len() != 1 {
                        errors.push(LoadError::new(file, lineno, "include takes exactly one path pattern"));
                    } else {
                        out.entries.push(Entry::Directive(Directive::Include { pattern: pattern[0].to_string(), line: lineno }));
                    }
                    continue;
                }
                Some("namespace") => {
                    let name: Vec<&str> = words.collect();
                    if seen_definition || out.namespace.is_some() {
                        errors.push(LoadError::new(file, lineno, "`namespace` must be the first line of the file, and only once"));
                    } else if name.len() != 1 || !crate::names::is_name(name[0]) {
                        errors.push(LoadError::new(file, lineno, "namespace takes one NAME"));
                    } else {
                        out.namespace = Some(name[0].to_string());
                    }
                    continue;
                }
                _ => {}
            }
        }

        if code_trimmed == "run:" {
            match current.as_ref() {
                Some(b) if b.kind != "command" => {
                    errors.push(LoadError::new(file, lineno, format!("`run:` belongs to a command block, not `{}`", b.kind)))
                }
                Some(b) if b.script.is_some() => errors.push(LoadError::new(file, lineno, "duplicate `run:`")),
                Some(_) if state != State::Outside => state = State::AwaitScript,
                _ => errors.push(LoadError::new(file, lineno, "`run:` outside a block")),
            }
            continue;
        }

        if code_trimmed == "legend:" {
            match (current.as_mut(), state) {
                (Some(b), _) if !b.has_legend => {
                    b.has_legend = true;
                    state = State::Legend;
                }
                (Some(_), _) => errors.push(LoadError::new(file, lineno, "duplicate `legend:`")),
                (None, _) => errors.push(LoadError::new(file, lineno, "`legend:` outside a block")),
            }
            continue;
        }

        let item_text = code_trimmed.strip_prefix("- ").or_else(|| (code_trimmed == "-").then_some(""));
        let is_text_block = current.as_ref().is_some_and(|b| b.kind == "text");

        match (state, current.as_mut()) {
            (State::Body | State::Legend | State::AwaitDrawing, Some(block)) => {
                if let Some(rest) = item_text {
                    match RawItem::new(file, lineno, out.namespace.clone(), rest) {
                        Ok(item) if item.tokens.is_empty() => errors.push(item.err("empty item")),
                        Ok(item) if state == State::Legend => block.legend.push(item),
                        Ok(item) => block.items.push(item),
                        Err(e) => errors.push(e),
                    }
                } else if is_text_block && state != State::Legend {
                    // bare `KEY text` line: the text is taken verbatim
                    match RawItem::new(file, lineno, out.namespace.clone(), trimmed) {
                        Ok(item) => block.items.push(item),
                        Err(e) => errors.push(e),
                    }
                } else {
                    errors.push(LoadError::new(file, lineno, format!("unexpected line in `{}` block: `{trimmed}`", block.kind)));
                }
            }
            (State::Closed, Some(block)) if item_text.is_some() => errors.push(LoadError::new(
                file,
                lineno,
                format!("item after a blank line: the `{}` block ended at the blank line above", block.kind),
            )),
            _ => {
                let word = code_trimmed.split_whitespace().next().unwrap_or("");
                let msg = if item_text.is_some() {
                    "item outside a block".to_string()
                } else if line.starts_with(char::is_whitespace) {
                    format!("unexpected text `{trimmed}`")
                } else if word == "pad" {
                    "gamepad `pad` blocks are not supported by Termplate (spec §9)".to_string()
                } else {
                    format!("unknown block kind `{word}`")
                };
                errors.push(LoadError::new(file, lineno, msg));
            }
        }
    }
    if matches!(state, State::FencedDrawing | State::SkipFence | State::Script(_)) {
        errors.push(LoadError::new(file, src.lines().count(), "unterminated code block"));
    }
    if state == State::AwaitScript {
        errors.push(LoadError::new(file, src.lines().count(), "`run:` must be followed by a ``` code block holding the script"));
    }
    flush(&mut current, &mut out);
    if errors.is_empty() { Ok(out) } else { Err(errors) }
}

/// Parses a header-less language file (D 10): `KEY text` lines, `>`
/// comments and an optional `namespace` line first.
pub fn parse_language(file: &str, src: &str) -> Result<(Option<String>, Vec<RawItem>), Vec<LoadError>> {
    let mut items = Vec::new();
    let mut errors = Vec::new();
    let mut namespace = None;
    for (idx, line) in src.lines().enumerate() {
        let lineno = idx + 1;
        let trimmed = line.trim_end_matches('\r').trim();
        if trimmed.is_empty() || is_comment(trimmed) || (trimmed.starts_with('#') && !trimmed.starts_with("#!")) {
            continue;
        }
        if let Some(ns) = trimmed.strip_prefix("namespace ") {
            if !items.is_empty() || namespace.is_some() {
                errors.push(LoadError::new(file, lineno, "`namespace` must be the first line of the file, and only once"));
            } else {
                namespace = Some(ns.trim().to_string());
            }
            continue;
        }
        match RawItem::new(file, lineno, namespace.clone(), trimmed) {
            Ok(item) => items.push(item),
            Err(e) => errors.push(e),
        }
    }
    if errors.is_empty() { Ok((namespace, items)) } else { Err(errors) }
}

/// Splits `s` at top-level commas (outside quotes, parentheses and `$( )`).
pub fn split_top_level(s: &str, sep: char) -> Vec<&str> {
    let mut out = Vec::new();
    let mut depth = 0usize;
    let mut quote: Option<char> = None;
    let mut start = 0;
    let mut chars = s.char_indices().peekable();
    while let Some((i, c)) = chars.next() {
        match (quote, c) {
            (Some(q), c) if c == q => quote = None,
            (Some('"'), '\\') => {
                chars.next();
            }
            (Some(_), _) => {}
            (None, '"' | '\'') => quote = Some(c),
            (None, '(') => depth += 1,
            (None, ')') => depth = depth.saturating_sub(1),
            (None, c) if c == sep && depth == 0 => {
                out.push(&s[start..i]);
                start = i + c.len_utf8();
            }
            _ => {}
        }
    }
    out.push(&s[start..]);
    out
}

/// Finds the `)` matching the `(` at byte `open`.
fn matching_close(s: &str, open: usize) -> Option<usize> {
    let mut depth = 0usize;
    let mut quote: Option<char> = None;
    for (i, c) in s[open..].char_indices() {
        match (quote, c) {
            (Some(q), c) if c == q => quote = None,
            (Some(_), _) => {}
            (None, '"' | '\'') => quote = Some(c),
            (None, '(') => depth += 1,
            (None, ')') => {
                depth -= 1;
                if depth == 0 {
                    return Some(open + i);
                }
            }
            _ => {}
        }
    }
    None
}

fn parse_header(
    file: &str,
    lineno: usize,
    line: &str,
    modifier: Modifier,
    kind: &str,
    namespace: &Option<String>,
) -> Result<RawBlock, LoadError> {
    let err = |m: String| LoadError::new(file, lineno, m);
    // skip the modifier and kind words
    let mut rest = line.trim_start();
    let skip = if modifier == Modifier::None { 1 } else { 2 };
    for _ in 0..skip {
        let end = rest.find(char::is_whitespace).unwrap_or(rest.len());
        rest = rest[end..].trim_start();
    }
    let name_end = rest.find(|c: char| c.is_whitespace() || c == '(').unwrap_or(rest.len());
    let name = rest[..name_end].to_string();
    let mut after = &rest[name_end..];
    let mut params = None;
    if after.starts_with('(') {
        let close = matching_close(after, 0).ok_or_else(|| err("missing `)` in header parameters".into()))?;
        let inner = &after[1..close];
        let mut list = Vec::new();
        for p in split_top_level(inner, ',').into_iter().map(str::trim).filter(|p| !p.is_empty()) {
            let (n, ty) = match p.split_once(':') {
                Some((n, t)) => (n.trim(), Some(t.trim().to_string())),
                None => (p, None),
            };
            if n.is_empty() || !n.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
                return Err(err(format!("bad parameter `{p}`")));
            }
            list.push(Param { name: n.to_string(), ty });
        }
        params = Some(list);
        after = &after[close + 1..];
    }
    if name.is_empty() || name.starts_with('"') {
        if crate::syntax::SECTION_KINDS.contains(&kind) && params.is_none() {
            // unnamed section
        } else {
            return Err(err(format!("`{kind}` block needs a name")));
        }
    }
    let args_text = after.trim().to_string();
    let args = tokenize(&args_text).map_err(|e| err(e.0))?;
    Ok(RawBlock {
        modifier,
        kind: kind.to_string(),
        name,
        params,
        args,
        args_text,
        file: file.to_string(),
        line: lineno,
        namespace: namespace.clone(),
        drawing: Vec::new(),
        drawing_line: 0,
        items: Vec::new(),
        legend: Vec::new(),
        has_legend: false,
        script: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn blocks(src: &str) -> Vec<RawBlock> {
        parse("t.termplate", src).unwrap_or_else(|e| panic!("{e:?}")).blocks().cloned().collect()
    }

    fn errs(src: &str) -> Vec<LoadError> {
        parse("t", src).unwrap_err()
    }

    #[test]
    fn parses_app_block_and_includes() {
        let src = "app BACKUPTOOL\n- termplate 0.1\n- layout MAIN\n\ninclude ui/*\ninclude text/*\n";
        let pf = parse("app.termplate", src).unwrap();
        assert_eq!(pf.entries.len(), 3);
        let Entry::Block(b) = &pf.entries[0] else { panic!() };
        assert_eq!(b.kind, "app");
        assert_eq!(b.name, "BACKUPTOOL");
        assert_eq!(b.items.len(), 2);
        assert_eq!(b.items[0].key(), "termplate");
        assert_eq!(b.items[1].line, 3);
        assert_eq!(pf.entries[1], Entry::Directive(Directive::Include { pattern: "ui/*".into(), line: 5 }));
    }

    #[test]
    fn unnamed_sections_and_comments() {
        let b = blocks("> header comment\nvars\n> inner\n- A text \"x\"\n");
        assert_eq!(b.len(), 1);
        assert_eq!(b[0].kind, "vars");
        assert_eq!(b[0].name, "");
        assert_eq!(b[0].items.len(), 1);
    }

    #[test]
    fn gt_without_space_is_not_a_comment() {
        let e = errs("menu M\n>oops\n");
        assert!(e[0].message.contains("unexpected line"), "{e:?}");
    }

    #[test]
    fn named_blocks_need_a_name() {
        assert!(errs("menu\n- A\n")[0].message.contains("needs a name"));
    }

    #[test]
    fn blank_line_ends_a_block() {
        let e = errs("menu M\n- A\n\n- B\n");
        assert!(e[0].message.contains("ended at the blank line"), "{e:?}");
        assert_eq!(e[0].line, 4);
        // a new header after the blank line is fine
        assert_eq!(blocks("menu M\n- A\n\nmenu N\n- B\n").len(), 2);
    }

    #[test]
    fn layout_drawing_and_legend_over_blank_lines() {
        let src = "layout MAIN\nsssmm\nsssmm\nlllll\n\nlegend:\n- s STATUS size.width 30\n\n- m MAIN_MENU\n";
        let b = &blocks(src)[0];
        assert_eq!(b.drawing, ["sssmm", "sssmm", "lllll"]);
        assert_eq!(b.drawing_line, 2);
        assert_eq!(b.legend.len(), 2, "legend continues over a blank line");
        assert!(b.items.is_empty());
    }

    #[test]
    fn drawing_may_follow_header_after_blank_line() {
        let b = &blocks("panel P\n\nab\n\nlegend:\n- a value $A\n")[0];
        assert_eq!(b.drawing, ["ab"]);
        assert_eq!(b.drawing_line, 3);
    }

    #[test]
    fn fenced_drawing_keeps_blank_lines_and_hashes() {
        let src = "panel P\n```\n#####\n\n# ab #\n```\n\nlegend:\n- a value $A\n";
        let b = &blocks(src)[0];
        assert_eq!(b.drawing, ["#####", "", "# ab #"]);
        assert_eq!(b.drawing_line, 3);
        assert_eq!(b.legend.len(), 1);
    }

    #[test]
    fn stray_code_blocks_are_errors() {
        assert!(errs("menu M\n- A\n```\nx\n```\n")[0].message.contains("must hold the drawing"));
        assert!(errs("```roguescript\nfn x()\n```\n")[0].message.contains("not supported"));
        assert!(errs("panel P\n```\nab\n")[0].message.contains("unterminated"));
    }

    #[test]
    fn markdown_headings_are_skipped() {
        let b = blocks("# Backup tool\n\n## Menus\nmenu M\n- A\n");
        assert_eq!(b.len(), 1);
    }

    #[test]
    fn panel_drawing_keeps_dash_and_gt_lines() {
        let src = "panel P refresh 2s\n+---+\n- x |\n> y |\n+---+\nlegend:\n- x value $A\n";
        let b = &blocks(src)[0];
        assert_eq!(b.drawing, ["+---+", "- x |", "> y |", "+---+"]);
        assert_eq!(b.args[0].text, "refresh");
        assert_eq!(b.args_text, "refresh 2s");
    }

    #[test]
    fn trailing_emphasis_comments() {
        assert_eq!(strip_emphasis("- HP now 8 max 8 ~~old value 6~~"), "- HP now 8 max 8");
        assert_eq!(strip_emphasis("entity GOBLIN **basic enemy**"), "entity GOBLIN");
        assert_eq!(strip_emphasis("- A ***x***"), "- A");
        assert_eq!(strip_emphasis("include content/**"), "include content/**");
        assert_eq!(strip_emphasis("- A ** spaced **"), "- A ** spaced **");
        assert_eq!(strip_emphasis("- A"), "- A");
        let b = blocks("menu M **main menu**\n- A ~~was B~~\n");
        assert_eq!(b[0].args_text, "");
        assert_eq!(b[0].items[0].text, "A");
    }

    #[test]
    fn header_parameters() {
        let b = &blocks("panel GRAVE(who: text, cause, gold: number) refresh 1s\nab\n\nlegend:\n- a param who\n- b param gold\n")[0];
        assert_eq!(b.name, "GRAVE");
        let p = b.params.as_ref().unwrap();
        assert_eq!(p[0], Param { name: "who".into(), ty: Some("text".into()) });
        assert_eq!(p[1], Param { name: "cause".into(), ty: None });
        assert_eq!(b.args_text, "refresh 1s");
        let b = &blocks("menu SHOP.BUY(wares: list) list size 36x*\n- source $wares\n")[0];
        assert_eq!(b.name, "SHOP.BUY");
        assert_eq!(b.args_text, "list size 36x*");
        assert!(errs("panel P(a b)\nx\n")[0].message.contains("bad parameter"));
        assert!(errs("panel P(a\nx\n")[0].message.contains("missing `)`"));
    }

    #[test]
    fn modifiers_on_headers() {
        let b = blocks("patch keys MENU\n- confirm enter\n\nnew menu X\n- A\n");
        assert_eq!(b[0].modifier, Modifier::Patch);
        assert_eq!(b[0].kind, "keys");
        assert_eq!(b[1].modifier, Modifier::New);
    }

    #[test]
    fn text_block_accepts_bare_lines() {
        let b = &blocks("text fi\nMAIN_MENU.BACKUP Varmuuskopioi\n- MAIN_MENU.QUIT Lopeta\n")[0];
        assert_eq!(b.items.len(), 2);
        assert_eq!(b.items[0].key(), "MAIN_MENU.BACKUP");
        assert_eq!(b.items[0].rest(1), "Varmuuskopioi");
    }

    #[test]
    fn header_args_text_for_layout_variant() {
        let b = &blocks("layout MAIN.narrow when width < 80\nab\n\nlegend:\n- a X\n- b Y\n")[0];
        assert_eq!(b.name, "MAIN.narrow");
        assert_eq!(b.args_text, "when width < 80");
    }

    #[test]
    fn unknown_block_kind_is_error() {
        let e = errs("widget FOO\n- a b\n");
        assert!(e[0].message.contains("unknown block kind `widget`"));
        assert_eq!(e[0].line, 1);
        assert!(e[1].message.contains("item outside a block"));
    }

    #[test]
    fn namespace_rules() {
        let pf = parse("t", "> c\nnamespace TOOLS\nmenu M\n- A\n").unwrap();
        assert_eq!(pf.namespace.as_deref(), Some("TOOLS"));
        assert_eq!(pf.blocks().next().unwrap().namespace.as_deref(), Some("TOOLS"));
        assert_eq!(pf.blocks().next().unwrap().items[0].namespace.as_deref(), Some("TOOLS"));
        assert!(errs("menu M\n- A\n\nnamespace X\n")[0].message.contains("first line"));
        assert!(errs("namespace X\nnamespace Y\n")[0].message.contains("only once"));
        assert!(errs("namespace lower\n")[0].message.contains("NAME"));
    }

    #[test]
    fn indented_keyword_is_not_header() {
        assert!(errs("menu M\n  menu X\n")[0].message.contains("unexpected line"));
    }

    #[test]
    fn language_files() {
        let (ns, items) = parse_language("fi.rlml", "> Finnish\nnamespace UNDEAD\n# heading\nPAUSE.TITLE Tauko\nINTRO.START Aloita **heti** {^confirm}.\n").unwrap();
        assert_eq!(ns.as_deref(), Some("UNDEAD"));
        assert_eq!(items[0].key(), "PAUSE.TITLE");
        assert_eq!(items[1].rest(1), "Aloita **heti** {^confirm}.", "no emphasis comments in language files");
        assert!(parse_language("fi", "A a\nnamespace X\n").is_err());
    }

    #[test]
    fn top_level_split() {
        assert_eq!(split_top_level("a, $(cut -d, -f1), \"x,y\"", ','), ["a", " $(cut -d, -f1)", " \"x,y\""]);
        assert_eq!(split_top_level("", ','), [""]);
    }

    #[test]
    fn run_script_sub_block() {
        let src = "command BACKUP\n- shell bash\nrun:\n```bash\n# comment\n\n  echo \"$X\" > out\n- not an item\n```\n- then quit\n";
        let b = &blocks(src)[0];
        let s = b.script.as_ref().unwrap();
        assert_eq!(s.lang, "bash");
        assert_eq!(s.line, 5);
        assert_eq!(s.lines, ["# comment", "", "  echo \"$X\" > out", "- not an item"]);
        assert_eq!(s.text(), "# comment\n\n  echo \"$X\" > out\n- not an item\n");
        assert_eq!(b.items.len(), 2, "items continue after the script");
        // after a blank line, with a longer fence and no language
        let b = &blocks("command C\n- output LOG\n\nrun:\n\n````\n```\n````\n")[0];
        assert_eq!(b.script.as_ref().unwrap().lines, ["```"]);
        assert_eq!(b.script.as_ref().unwrap().lang, "");
    }

    #[test]
    fn run_script_errors() {
        assert!(errs("command C\nrun:\necho hi\n")[0].message.contains("must be followed"));
        assert!(errs("command C\nrun:\n")[0].message.contains("must be followed"));
        assert!(errs("menu M\nrun:\n```\nx\n```\n")[0].message.contains("belongs to a command"));
        assert!(errs("command C\nrun:\n```\nx\n")[0].message.contains("unterminated"));
        assert!(errs("command C\nrun:\n```\nx\n```\nrun:\n```\ny\n```\n")[0].message.contains("duplicate"));
    }

    #[test]
    fn shift_drops_words() {
        let i = RawItem::new("f", 1, None, "new pause p").unwrap();
        let s = i.shift(1);
        assert_eq!(s.text, "pause p");
        assert_eq!(s.key(), "pause");
        assert_eq!(s.span(0, 2), "pause p");
    }
}
