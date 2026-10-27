//! The typed application model and load-time validation (spec §1–§13,
//! RLML D 10, D 11).
//!
//! [`Model::build`] turns merged raw blocks into typed definitions,
//! resolves every reference through the namespace lookup (D 4.4), lets the
//! host check every expression and reports all problems together.

use std::collections::{BTreeMap, BTreeSet};
use std::time::Duration;

use crate::error::{LoadError, LoadErrors};
use crate::host::{Expr, ExprKind, Host};
use crate::keys::{action_alias, parse_binding, parse_pad, Binding, Keymap, Mode, BUILTIN_CONTEXTS, KEY_ACTIONS};
use crate::layout::{analyze, AreaSize, Dim, Dims, Grid, Rect, SizeCond};
use crate::names::{self, is_name, is_name_with_variant};
use crate::panel;
use crate::style::{Border, StyleSheet, VARIANTS};
use crate::syntax::{split_top_level, Param, RawBlock, RawItem};
use crate::text::Texts;
use crate::token::Token;
use crate::value::{parse_range, Value, VarType};

pub const SUPPORTED_VERSIONS: &[&str] = &["0.1"];

#[derive(Debug, Clone, PartialEq)]
pub struct App {
    pub name: String,
    pub version: String,
    pub layout: Option<String>,
    pub language: Option<String>,
    pub workdir: Option<String>,
    /// Base key context (D 3.3 `keys`); defaults to the layout name.
    pub keys: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct VarDef {
    pub name: String,
    pub ty: VarType,
    pub default: Value,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Align {
    #[default]
    Left,
    Center,
    Right,
}

impl Align {
    fn parse(s: &str) -> Option<Align> {
        match s {
            "left" => Some(Align::Left),
            "center" => Some(Align::Center),
            "right" => Some(Align::Right),
            _ => None,
        }
    }
}

/// A call argument: positional (`None`) or named.
pub type Arg = (Option<String>, Expr);

#[derive(Debug, Clone, PartialEq)]
pub enum Action {
    Quit,
    Back,
    Close,
    Confirm,
    Refresh,
    NextArea,
    PrevArea,
    /// `focus N`: the N:th numbered area (spec §4).
    Focus(usize),
    /// Cursor movement and `select` (keys only).
    Nav(String),
    Open { target: String, args: Vec<Arg> },
    Set { var: String, value: Expr },
    Toggle(String),
    /// Shows or hides a layout panel (D 11.3 `toggle PANEL`).
    TogglePanel(String),
    Adjust { var: String, lo: f64, hi: f64, pool: Option<String> },
    Emit(String),
    /// `stop COMMAND`: cancels the command when it runs (spec §6).
    Stop(String),
    Command(String),
    Run { cmdline: String, interactive: bool },
}

impl Action {
    /// The built-in action named by a lower-case word.
    pub fn builtin(word: &str) -> Option<Action> {
        Some(match word {
            "quit" => Action::Quit,
            "back" => Action::Back,
            "close" => Action::Close,
            "confirm" => Action::Confirm,
            "refresh" => Action::Refresh,
            "next_area" => Action::NextArea,
            "prev_area" => Action::PrevArea,
            n if KEY_ACTIONS.contains(&n) => Action::Nav(n.to_string()),
            _ => return None,
        })
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct AreaDef {
    pub target: String,
    pub args: Vec<Arg>,
    pub width: Option<Dim>,
    pub height: Option<Dim>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct LayoutDef {
    pub name: String,
    pub base: String,
    pub when: Option<SizeCond>,
    pub grid: Grid,
    pub areas: BTreeMap<char, AreaDef>,
    /// Legend characters in legend order (area numbering, spec §4).
    pub order: Vec<char>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum FieldKind {
    Text { key: String, params: Vec<(String, Expr)> },
    Value { expr: Expr },
    Param { name: String },
    Bar { value: Expr, max: Expr },
    /// Editable field; `input` names an `input` block (an implicit text
    /// input is created for a bare variable).
    Input { input: String },
    Spinner { when: Expr },
}

#[derive(Debug, Clone, PartialEq)]
pub struct FieldDef {
    pub name: String,
    pub kind: FieldKind,
    pub align: Align,
    pub rects: Vec<Rect>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PanelDef {
    pub name: String,
    pub params: Vec<Param>,
    pub refresh: Option<Duration>,
    pub picture: Vec<Vec<char>>,
    pub width: usize,
    pub height: usize,
    pub fields: Vec<FieldDef>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SecretMode {
    Stdin,
    Env,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CommandDef {
    pub name: String,
    pub confirm: Option<String>,
    pub run: Option<String>,
    pub interactive: bool,
    pub env: Vec<(String, Expr)>,
    pub output: Option<String>,
    pub then: Option<Action>,
    pub on_error: Option<Action>,
    pub into: Option<String>,
    pub secrets: Vec<(String, SecretMode)>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct MenuItem {
    pub key: String,
    pub action: Action,
    pub hide_when: Option<Expr>,
    pub disabled_when: Option<Expr>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Column {
    pub name: String,
    /// `None` = `*` (fill).
    pub width: Option<usize>,
    pub align: Align,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ListDef {
    pub source: Expr,
    /// `row NAME: expr, …`: parameters of `<MENU>.ROW`, or column values.
    pub row: Vec<(String, Expr)>,
    pub sort: Option<(Expr, bool)>,
    pub filter: bool,
    pub multi: bool,
    pub keep: bool,
    pub then: Option<Action>,
    pub hide_when: Option<Expr>,
    pub disabled_when: Option<Expr>,
    pub columns: Vec<Column>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum MenuKind {
    Static(Vec<MenuItem>),
    List(Box<ListDef>),
}

#[derive(Debug, Clone, PartialEq)]
pub struct MenuDef {
    pub name: String,
    pub params: Vec<Param>,
    pub kind: MenuKind,
    /// Key context; `MENU` when not given (D 11.2).
    pub context: Option<String>,
    pub size: (Option<Dim>, Option<Dim>),
    /// `final`: `back` and `close` do not close it (D 11.2).
    pub final_: bool,
}

impl MenuDef {
    pub fn context(&self) -> &str {
        self.context.as_deref().unwrap_or("MENU")
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputKind {
    Text,
    Number,
    Password,
    Path,
    Choice,
}

#[derive(Debug, Clone, PartialEq)]
pub struct InputDef {
    pub name: String,
    pub var: String,
    pub kind: InputKind,
    pub validate: Option<Expr>,
    pub error: Option<String>,
    /// Runs after a valid value was stored (spec §8).
    pub then: Option<Action>,
    /// `clear`: empties the field after `then` (a chat or command line).
    pub clear: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct LogDef {
    pub name: String,
    pub history: usize,
    pub merge: bool,
    pub more: bool,
    pub keep_ansi: bool,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Settings {
    pub language: Option<String>,
    pub theme: Option<String>,
    pub refresh_factor: Option<f64>,
    pub timeout: Option<Duration>,
    pub mouse: Option<bool>,
    /// Shows area numbers in area borders (spec §4).
    pub area_numbers: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Model {
    pub app: App,
    pub vars: Vec<VarDef>,
    /// Base layout name → base definition followed by its variants.
    pub layouts: BTreeMap<String, Vec<LayoutDef>>,
    pub panels: BTreeMap<String, PanelDef>,
    pub commands: BTreeMap<String, CommandDef>,
    pub menus: BTreeMap<String, MenuDef>,
    pub inputs: BTreeMap<String, InputDef>,
    pub logs: BTreeMap<String, LogDef>,
    pub keymap: Keymap,
    pub texts: Texts,
    pub style: StyleSheet,
    pub settings: Settings,
    pub warnings: Vec<LoadError>,
}

pub fn is_var_name(s: &str) -> bool {
    !s.contains('.') && is_name(s)
}

pub fn parse_duration(s: &str) -> Option<Duration> {
    let (num, unit) = s.find(|c: char| !c.is_ascii_digit() && c != '.').map(|i| s.split_at(i))?;
    let n: f64 = num.parse().ok()?;
    let secs = match unit {
        "ms" => n / 1000.0,
        "s" => n,
        "m" => n * 60.0,
        _ => return None,
    };
    Some(Duration::from_secs_f64(secs))
}

/// The base name of a layout: its upper-case parts (`MAIN.narrow` → `MAIN`).
pub fn base_layout(name: &str) -> &str {
    let mut end = 0;
    for (i, part) in name.split('.').enumerate() {
        if !part.starts_with(|c: char| c.is_ascii_uppercase()) {
            break;
        }
        end += part.len() + usize::from(i > 0);
    }
    &name[..end]
}

const DEF_KINDS: &[&str] = &["layout", "panel", "menu", "input", "command", "log"];
const RESERVED_ITEMS: &[&str] = &["TITLE", "TEXT", "ROW", "EMPTY"];

struct Builder<'h> {
    host: &'h mut dyn Host,
    errors: LoadErrors,
    warnings: Vec<LoadError>,
    /// full name → kind
    defs: BTreeMap<String, &'static str>,
    params: BTreeMap<String, Vec<Param>>,
    contexts: BTreeSet<String>,
    vars: BTreeMap<String, VarType>,
    texts: Texts,
    text_locs: BTreeMap<(String, String), (String, usize)>,
    implicit_inputs: Vec<InputDef>,
}

impl Builder<'_> {
    fn err(&mut self, e: LoadError) {
        self.errors.push(e);
    }

    fn expr(&mut self, src: &str, item: &RawItem, kind: ExprKind) -> Expr {
        let e = Expr::new(src.trim(), &item.file, item.line);
        if src.trim().is_empty() {
            self.errors.push(item.err("missing expression"));
        } else if let Err(msg) = self.host.check(&e, kind) {
            self.errors.push(item.err(msg));
        }
        e
    }

    /// Resolves a reference to a definition of one of `kinds` (D 4.4): the
    /// lookup finds the name first and checks its kind afterwards.
    fn resolve(&mut self, item: &RawItem, name: &str, kinds: &[&str]) -> Option<String> {
        let what = kinds.join(" or ");
        match names::resolve(item.namespace.as_deref(), name, |n| self.defs.contains_key(n)) {
            Ok(full) => {
                let kind = self.defs[&full];
                if kinds.contains(&kind) {
                    Some(full)
                } else {
                    self.err(item.err(format!("`{name}` is a {kind}, not a {what}")));
                    None
                }
            }
            Err(tried) => {
                self.err(item.err(format!("unknown {what} `{name}` (tried {tried})")));
                None
            }
        }
    }

    fn try_resolve(&self, item: &RawItem, name: &str, kinds: &[&str]) -> Option<String> {
        names::resolve(item.namespace.as_deref(), name, |n| self.defs.get(n).is_some_and(|k| kinds.contains(k))).ok()
    }

    /// A text key reference: the first candidate that has a text, else the
    /// key as written (a missing text shows as its key).
    fn text_key(&self, item: &RawItem, key: &str) -> String {
        names::candidates(item.namespace.as_deref(), key)
            .into_iter()
            .find(|k| self.texts.has(k))
            .unwrap_or_else(|| key.trim_start_matches('.').to_string())
    }
}

impl Model {
    pub fn build(blocks: Vec<RawBlock>, host: &mut dyn Host) -> Result<Model, LoadErrors> {
        let mut bld = Builder {
            host,
            errors: LoadErrors::default(),
            warnings: vec![],
            defs: BTreeMap::new(),
            params: BTreeMap::new(),
            contexts: BUILTIN_CONTEXTS.iter().map(|s| s.to_string()).collect(),
            vars: BTreeMap::new(),
            texts: Texts::default(),
            text_locs: BTreeMap::new(),
            implicit_inputs: vec![],
        };
        for b in &blocks {
            let Some(kind) = DEF_KINDS.iter().find(|k| **k == b.kind) else { continue };
            let ok = if b.kind == "layout" { is_name_with_variant(&b.name) } else { is_name(&b.name) };
            if !ok {
                bld.err(b.err(format!("invalid name `{}` (UPPER_CASE parts joined with dots)", b.name)));
            }
            let name = if b.kind == "layout" { base_layout(&b.name).to_string() } else { b.name.clone() };
            bld.defs.insert(name.clone(), kind);
            if let Some(p) = &b.params {
                bld.params.insert(name, p.clone());
            }
            if b.kind == "menu"
                && let Some(i) = b.args.iter().position(|t| t.is("context"))
                    && let Some(c) = b.args.get(i + 1) {
                        bld.contexts.insert(c.text.clone());
                    }
        }

        let apps: Vec<&RawBlock> = blocks.iter().filter(|b| b.kind == "app").collect();
        let mut app = match apps.as_slice() {
            [] => {
                let file = blocks.first().map(|b| b.file.clone()).unwrap_or_default();
                return Err(LoadError::new(file, 0, "missing `app` block").into());
            }
            [one] => build_app(&mut bld, one),
            [_, second, ..] => {
                bld.err(second.err("only one `app` block is allowed"));
                build_app(&mut bld, apps[0])
            }
        };
        let layout_names: BTreeSet<String> =
            blocks.iter().filter(|b| b.kind == "layout").map(|b| base_layout(&b.name).to_string()).collect();
        match &app.layout {
            Some(l) if layout_names.contains(l) => {}
            Some(l) => bld.err(apps[0].err(format!("unknown layout `{l}`"))),
            None if layout_names.len() == 1 => app.layout = layout_names.first().cloned(),
            None if layout_names.len() > 1 => bld.err(apps[0].err(format!(
                "several layouts and no `- layout` in the app block: {}",
                layout_names.iter().cloned().collect::<Vec<_>>().join(", ")
            ))),
            None => {}
        }
        let base_ctx = app.keys.clone().or_else(|| app.layout.clone());
        bld.contexts.extend(base_ctx.iter().cloned());
        bld.contexts.extend(layout_names.iter().cloned());

        let mut vars = Vec::new();
        for b in blocks.iter().filter(|b| b.kind == "vars") {
            for item in &b.items {
                if let Some(v) = build_var(&mut bld, item) {
                    vars.push(v);
                }
            }
        }
        bld.vars = vars.iter().map(|v: &VarDef| (v.name.clone(), v.ty.clone())).collect();

        // Texts first: other blocks resolve text keys against them.
        for b in blocks.iter().filter(|b| b.kind == "text") {
            build_text(&mut bld, b);
        }
        let mut style = StyleSheet::default();
        for b in blocks.iter().filter(|b| b.kind == "colors") {
            build_colors(&mut bld, b, &mut style);
        }

        let mut model = Model {
            app,
            vars,
            layouts: BTreeMap::new(),
            panels: BTreeMap::new(),
            commands: BTreeMap::new(),
            menus: BTreeMap::new(),
            inputs: BTreeMap::new(),
            logs: BTreeMap::new(),
            keymap: Keymap::with_defaults(),
            texts: Texts::default(),
            style,
            settings: Settings::default(),
            warnings: vec![],
        };

        for b in &blocks {
            match b.kind.as_str() {
                "app" | "vars" | "text" | "colors" => {}
                "layout" => {
                    if let Some(l) = build_layout(&mut bld, b) {
                        model.layouts.entry(l.base.clone()).or_default().push(l);
                    }
                }
                "panel" => {
                    if let Some(p) = build_panel(&mut bld, b) {
                        model.panels.insert(p.name.clone(), p);
                    }
                }
                "command" => {
                    let c = build_command(&mut bld, b);
                    model.commands.insert(c.name.clone(), c);
                }
                "menu" => {
                    let m = build_menu(&mut bld, b);
                    model.menus.insert(m.name.clone(), m);
                }
                "input" => {
                    let i = build_input(&mut bld, b);
                    model.inputs.insert(i.name.clone(), i);
                }
                "log" => {
                    let l = build_log(&mut bld, b);
                    model.logs.insert(l.name.clone(), l);
                }
                "keys" | "pad" => build_keys(&mut bld, b, &mut model.keymap),
                "panels" | "logs" => build_style_rows(&mut bld, b, &mut model.style),
                "settings" => model.settings = build_settings(&mut bld, b),
                _ => unreachable!("syntax only yields known kinds"),
            }
        }
        for inp in std::mem::take(&mut bld.implicit_inputs) {
            model.inputs.entry(inp.name.clone()).or_insert(inp);
        }
        for layouts in model.layouts.values_mut() {
            layouts.sort_by_key(|l| l.name != l.base);
        }
        // A wide character takes two columns but one drawing cell (M 2.5).
        for b in blocks.iter().filter(|b| !b.drawing.is_empty()) {
            for (i, line) in b.drawing.iter().enumerate() {
                if let Some(c) = line.chars().find(|c| crate::frame::char_width(*c) > 1) {
                    let msg = format!("wide character `{c}` in the drawing of {} {}: it takes two columns", b.kind, b.name);
                    bld.warnings.push(LoadError::new(&b.file, b.drawing_line + i, msg));
                }
            }
        }
        // Borders come from the style, not the drawing (spec §5, §11).
        for b in blocks.iter().filter(|b| b.kind == "panel" && b.drawing.len() >= 2) {
            let edge = |l: &String| l.chars().count() >= 2 && l.chars().all(|c| "+-=─═━┌┐└┘╔╗╚╝╭╮╰╯┏┓┗┛".contains(c));
            if edge(&b.drawing[0]) && b.drawing.last().is_some_and(edge) {
                let msg = format!("panel {} draws its own border: borders come from the style (`panels - {} border single`)", b.name, b.name);
                bld.warnings.push(LoadError::new(&b.file, b.drawing_line, msg));
            }
        }
        model.keymap.add_focus_defaults();
        let app_file = apps[0].file.clone();
        for (ctx, key) in model.keymap.always_shadowing() {
            bld.warnings.push(LoadError::new(&app_file, 0, format!("key `{key}` is bound in ALWAYS, so its binding in {ctx} never fires")));
        }
        check_texts(&mut bld, &model);
        model.app.keys = base_ctx;
        let mut texts = std::mem::take(&mut bld.texts);
        texts.fallback_language = model.app.language.clone();
        texts.language = model.settings.language.clone().or_else(|| model.app.language.clone());
        model.texts = texts;
        model.warnings = bld.warnings;
        bld.errors.into_result(model)
    }

    pub fn var(&self, name: &str) -> Option<&VarDef> {
        self.vars.iter().find(|v| v.name == name)
    }

    /// Picks the layout variant for a terminal size (spec §4): the first
    /// variant whose `when` holds; otherwise the base layout if it fits;
    /// otherwise the first variant that fits; otherwise the base.
    pub fn choose_layout(&self, base: &str, width: usize, height: usize) -> Option<&LayoutDef> {
        let defs = self.layouts.get(base)?;
        if let Some(v) = defs.iter().find(|d| d.when.as_ref().is_some_and(|c| c.eval(width, height))) {
            return Some(v);
        }
        let fits = |d: &LayoutDef| {
            let (w, h) = d.grid.min_size(&self.area_sizes(d, width, height));
            width >= w && height >= h
        };
        let base_def = defs.iter().find(|d| d.name == base)?;
        if fits(base_def) {
            return Some(base_def);
        }
        defs.iter().find(|d| d.name != base && d.when.is_none() && fits(d)).or(Some(base_def))
    }

    /// Area sizes in cells for a window of `width`×`height`. A styled
    /// border adds two cells to the content's minimum.
    pub fn area_sizes(&self, l: &LayoutDef, width: usize, height: usize) -> BTreeMap<char, AreaSize> {
        l.areas
            .iter()
            .map(|(c, a)| {
                let (mw, mh) = self.panels.get(&a.target).map(|p| (p.width, p.height)).unwrap_or((0, 0));
                let border = if self.area_border(&a.target).is_some() { 2 } else { 0 };
                let size = AreaSize {
                    width: a.width.map(|d| d.resolve(width)),
                    height: a.height.map(|d| d.resolve(height)),
                    min_width: mw + border,
                    min_height: mh + border,
                };
                (*c, size)
            })
            .collect()
    }

    /// The border a `panels` style row gives a layout area.
    pub fn area_border(&self, target: &str) -> Option<Border> {
        self.style.border(&[target.to_string()]).filter(|b| *b != Border::None)
    }
}

fn build_app(bld: &mut Builder, b: &RawBlock) -> App {
    let mut app = App { name: b.name.clone(), version: String::new(), layout: None, language: None, workdir: None, keys: None };
    if !is_name(&b.name) {
        bld.err(b.err(format!("invalid app name `{}`", b.name)));
    }
    for item in &b.items {
        let value = item.tokens.get(1).map(|t| t.text.clone());
        let Some(value) = value.filter(|_| item.tokens.len() == 2) else {
            bld.err(item.err(format!("`{}` takes one value", item.key())));
            continue;
        };
        match item.key() {
            "termplate" => {
                if !SUPPORTED_VERSIONS.contains(&value.as_str()) {
                    bld.err(item.err(format!("unsupported termplate version {value} (supported: {})", SUPPORTED_VERSIONS.join(", "))));
                }
                app.version = value;
            }
            "layout" => app.layout = Some(value),
            "language" => app.language = Some(value),
            "workdir" => app.workdir = Some(value),
            "keys" => app.keys = Some(value),
            other => bld.err(item.err(format!("unknown field `{other}` in app block"))),
        }
    }
    if !b.items.iter().any(|i| i.key() == "termplate") {
        bld.err(b.err("missing required field `- termplate VERSION` in app block"));
    }
    app
}

fn build_var(bld: &mut Builder, item: &RawItem) -> Option<VarDef> {
    let t = &item.tokens;
    let name = item.key().to_string();
    if !is_var_name(&name) {
        bld.err(item.err(format!("invalid variable name `{name}` (use UPPER_CASE)")));
        return None;
    }
    let Some(ty_tok) = t.get(1) else {
        bld.err(item.err(format!("variable `{name}` needs a type")));
        return None;
    };
    let mut rest: &[Token] = &t[2..];
    let ty = match ty_tok.text.as_str() {
        "text" => VarType::Text,
        "bool" => VarType::Bool,
        "list" => VarType::List,
        "number" => {
            let mut range = None;
            if let Some(i) = rest.iter().position(|x| x.is("range")) {
                match rest.get(i + 1).and_then(|r| parse_range(&r.text)) {
                    Some(r) if i + 2 == rest.len() => range = Some(r),
                    _ => {
                        bld.err(item.err("expected `range A..B` at the end"));
                        return None;
                    }
                }
                rest = &rest[..i];
            }
            VarType::Number { range }
        }
        "choice" => {
            let mut opts: Vec<String> = rest.iter().map(|x| x.text.clone()).collect();
            let mut default = None;
            if let Some(i) = opts.iter().position(|o| o == "default") {
                default = opts.get(i + 1).cloned();
                opts.truncate(i);
            }
            if opts.is_empty() {
                bld.err(item.err("choice needs options"));
                return None;
            }
            let ty = VarType::Choice(opts);
            let default = match default {
                Some(d) => match ty.parse_value(&d) {
                    Ok(v) => v,
                    Err(e) => {
                        bld.err(item.err(e));
                        return None;
                    }
                },
                None => ty.zero(),
            };
            return Some(VarDef { name, ty, default });
        }
        other => {
            bld.err(item.err(format!("unknown type `{other}` (text, number, bool, list, choice)")));
            return None;
        }
    };
    let default = match rest {
        [] => ty.zero(),
        [d] => match ty.parse_value(&d.text) {
            Ok(v) => v,
            Err(e) => {
                bld.err(item.err(format!("bad default for `{name}`: {e}")));
                return None;
            }
        },
        _ => {
            bld.err(item.err("too many words in variable definition"));
            return None;
        }
    };
    Some(VarDef { name, ty, default })
}

/// Parses call arguments `a, name: b` (RogueScript form, positional first).
fn parse_args(bld: &mut Builder, item: &RawItem, inner: &str) -> Vec<Arg> {
    let mut out: Vec<Arg> = Vec::new();
    for a in split_top_level(inner, ',').into_iter().map(str::trim).filter(|a| !a.is_empty()) {
        let named = a
            .split_once(':')
            .filter(|(n, v)| !n.is_empty() && n.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') && v.starts_with(' '));
        match named {
            Some((n, v)) => out.push((Some(n.to_string()), bld.expr(v, item, ExprKind::Value))),
            None => {
                if out.iter().any(|(n, _)| n.is_some()) {
                    bld.err(item.err("positional arguments must come before named ones"));
                }
                out.push((None, bld.expr(a, item, ExprKind::Value)));
            }
        }
    }
    out
}

/// Splits `NAME(args)` into the name and the argument text.
fn split_call(s: &str) -> Result<(&str, Option<&str>), String> {
    match s.split_once('(') {
        Some((name, rest)) => {
            let inner = rest.trim_end().strip_suffix(')').ok_or("missing `)`")?;
            Ok((name.trim(), Some(inner)))
        }
        None => Ok((s.trim(), None)),
    }
}

/// Checks arguments against a definition's header parameters (M 2.5).
fn check_args(bld: &mut Builder, item: &RawItem, target: &str, args: &[Arg]) {
    let params = bld.params.get(target).cloned().unwrap_or_default();
    if args.len() > params.len() {
        bld.err(item.err(format!("`{target}` takes {} argument(s), got {}", params.len(), args.len())));
        return;
    }
    for (name, _) in args.iter() {
        if let Some(n) = name
            && !params.iter().any(|p| p.name == *n) {
                bld.err(item.err(format!("`{target}` has no parameter `{n}`")));
            }
    }
}

/// `NAME: expr, NAME: expr` (D 11.2 `row`, D 11.5 text parameters).
fn parse_named(bld: &mut Builder, item: &RawItem, from: usize, to: usize) -> Vec<(String, Expr)> {
    let t = &item.tokens[..to];
    let starts: Vec<usize> = (from..to)
        .filter(|&i| {
            let w = &t[i].text;
            !t[i].quoted && w.len() > 1 && w.ends_with(':') && w[..w.len() - 1].chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
        })
        .collect();
    if starts.first() != Some(&from) {
        bld.err(item.err("expected `NAME: expression, …`"));
        return vec![];
    }
    let mut out = Vec::new();
    for (k, &s) in starts.iter().enumerate() {
        let end = starts.get(k + 1).copied().unwrap_or(to);
        let name = t[s].text.trim_end_matches(':').to_string();
        let src = item.span(s + 1, end).trim().trim_end_matches(',').trim();
        out.push((name, bld.expr(src, item, ExprKind::Value)));
    }
    out
}

fn build_layout(bld: &mut Builder, b: &RawBlock) -> Option<LayoutDef> {
    let base = base_layout(&b.name).to_string();
    let is_variant = base != b.name;
    let when = if b.args.is_empty() {
        None
    } else if b.args[0].is("when") && is_variant {
        match SizeCond::parse(b.args_text.trim_start_matches("when").trim()) {
            Ok(c) => Some(c),
            Err(e) => {
                bld.err(b.err(e));
                None
            }
        }
    } else {
        bld.err(b.err(format!("unexpected `{}` in layout header (variants use `layout NAME.variant when …`)", b.args_text)));
        None
    };
    let grid = match analyze(&b.drawing) {
        Ok(g) => g,
        Err(e) => {
            bld.err(LoadError::new(&b.file, b.drawing_line.max(b.line), e));
            return None;
        }
    };
    let mut areas = BTreeMap::new();
    let mut order = Vec::new();
    let mut dims = Dims::new();
    for item in &b.legend {
        let key = item.key();
        let mut chars = key.chars();
        let (Some(ch), None) = (chars.next(), chars.next()) else {
            bld.err(item.err(format!("layout legend key must be one character, got `{key}`")));
            continue;
        };
        if !grid.areas.contains_key(&ch) {
            bld.err(item.err(format!("legend `{ch}` does not appear in the layout drawing")));
            continue;
        }
        if item.tokens.len() < 2 {
            bld.err(item.err("legend entry needs a target (panel, menu, log or input)"));
            continue;
        }
        // `STATS(player, title: "x")` may span several words
        let mut opts_from = 2;
        while item.span(1, opts_from).matches('(').count() > item.span(1, opts_from).matches(')').count() && opts_from < item.tokens.len() {
            opts_from += 1;
        }
        let target_text = item.span(1, opts_from).to_string();
        let (target_name, arg_text) = match split_call(&target_text) {
            Ok(x) => x,
            Err(e) => {
                bld.err(item.err(e));
                continue;
            }
        };
        let target = bld.resolve(item, target_name, &["panel", "menu", "log", "input"]).unwrap_or_default();
        let args = arg_text.map(|a| parse_args(bld, item, a)).unwrap_or_default();
        if !target.is_empty() {
            check_args(bld, item, &target, &args);
        }
        let mut area = AreaDef { target, args, width: None, height: None };
        let mut i = opts_from;
        while i < item.tokens.len() {
            let word = item.tokens[i].text.as_str();
            let Some(value) = item.tokens.get(i + 1).map(|t| t.text.as_str()) else {
                bld.err(item.err(format!("`{word}` needs a value")));
                break;
            };
            let r = match word {
                "size" => Dim::parse_pair(value).map(|(w, h)| {
                    area.width = w;
                    area.height = h;
                }),
                "size.width" => Dim::parse(value).map(|d| area.width = d),
                "size.height" => Dim::parse(value).map(|d| area.height = d),
                _ => Err(format!("unknown area option `{word}` (size WxH, size.width N, size.height N)")),
            };
            if let Err(e) = r {
                bld.err(item.err(e));
                break;
            }
            i += 2;
        }
        dims.insert(ch, (area.width, area.height));
        if areas.insert(ch, area).is_some() {
            bld.err(item.err(format!("duplicate legend `{ch}`")));
        } else {
            order.push(ch);
        }
    }
    for ch in grid.areas.keys() {
        if !areas.contains_key(ch) {
            bld.err(b.err(format!("layout area `{ch}` has no legend entry")));
        }
    }
    if let Err(e) = grid.check_conflicts(&dims) {
        bld.err(b.err(e));
    }
    Some(LayoutDef { name: b.name.clone(), base, when, grid, areas, order })
}

/// Splits a trailing alignment word off `tokens[from..]`.
fn split_align(item: &RawItem, from: usize) -> (usize, Align) {
    let n = item.tokens.len();
    if n > from + 1 && !item.tokens[n - 1].quoted
        && let Some(a) = Align::parse(&item.tokens[n - 1].text) {
            return (n - 1, a);
        }
    (n, Align::Left)
}

fn build_panel(bld: &mut Builder, b: &RawBlock) -> Option<PanelDef> {
    let mut refresh = None;
    let mut i = 0;
    while i < b.args.len() {
        match (b.args[i].text.as_str(), b.args.get(i + 1)) {
            ("refresh", Some(d)) => match parse_duration(&d.text) {
                Some(d) => refresh = Some(d),
                None => bld.err(b.err(format!("bad duration `{}` (e.g. 2s, 500ms, 1m)", d.text))),
            },
            (w, _) => bld.err(b.err(format!("unknown panel option `{w}`"))),
        }
        i += 2;
    }
    if b.drawing.is_empty() {
        bld.err(b.err(format!("panel `{}` has no drawing", b.name)));
        return None;
    }
    let names: Vec<&str> = b.legend.iter().map(|i| i.key()).collect();
    let det = match panel::detect(&b.drawing, &names) {
        Ok(d) => d,
        Err(e) => {
            bld.err(LoadError::new(&b.file, b.drawing_line, e));
            return None;
        }
    };
    let params = b.params.clone().unwrap_or_default();
    let mut fields = Vec::new();
    for item in &b.legend {
        if fields.iter().any(|f: &FieldDef| f.name == item.key()) {
            bld.err(item.err(format!("duplicate legend `{}`", item.key())));
            continue;
        }
        let Some(kind_tok) = item.tokens.get(1) else {
            bld.err(item.err("legend entry needs a field kind (text, value, param, bar, input, spinner)"));
            continue;
        };
        let (end, align) = split_align(item, 2);
        let kind = match kind_tok.text.as_str() {
            "text" => {
                let Some(key_tok) = item.tokens.get(2).filter(|_| end >= 3) else {
                    bld.err(item.err("`text KEY`"));
                    continue;
                };
                let mut stop = end;
                if let Some(d) = item.tokens.get(end - 1).filter(|t| t.quoted && end > 3) {
                    // inline default text (Termplate)
                    let key = names::qualify(item.namespace.as_deref(), &key_tok.text);
                    bld.texts.defaults.insert(key, d.text.clone());
                    stop = end - 1;
                }
                let key = bld.text_key(item, &key_tok.text);
                let tparams = if stop > 3 { parse_named(bld, item, 3, stop) } else { vec![] };
                FieldKind::Text { key, params: tparams }
            }
            "value" => FieldKind::Value { expr: bld.expr(item.span(2, end), item, ExprKind::Value) },
            "param" => match item.tokens.get(2) {
                Some(p) if end == 3 => {
                    if !params.iter().any(|x| x.name == p.text) {
                        bld.err(item.err(format!("`{}` is not a parameter of panel {}", p.text, b.name)));
                    }
                    FieldKind::Param { name: p.text.clone() }
                }
                _ => {
                    bld.err(item.err("`param NAME`"));
                    continue;
                }
            },
            "bar" => {
                let Some(of) = item.tokens[..end].iter().position(|t| t.is("of")) else {
                    bld.err(item.err("`bar VALUE of MAX`"));
                    continue;
                };
                let value = bld.expr(item.span(2, of), item, ExprKind::Value);
                let max = bld.expr(item.span(of + 1, end), item, ExprKind::Value);
                FieldKind::Bar { value, max }
            }
            "input" => match item.tokens.get(2) {
                Some(t) if end == 3 => {
                    let name = t.text.clone();
                    if let Some(full) = bld.try_resolve(item, &name, &["input"]) {
                        FieldKind::Input { input: full }
                    } else if bld.vars.contains_key(&name) {
                        bld.implicit_inputs.push(InputDef { name: name.clone(), var: name.clone(), kind: InputKind::Text, validate: None, error: None, then: None, clear: false });
                        FieldKind::Input { input: name }
                    } else {
                        bld.err(item.err(format!("unknown input or variable `{name}`")));
                        continue;
                    }
                }
                _ => {
                    bld.err(item.err("`input VARIABLE`"));
                    continue;
                }
            },
            "spinner" => {
                if !item.tokens.get(2).is_some_and(|t| t.is("when")) {
                    bld.err(item.err("`spinner when CONDITION`"));
                    continue;
                }
                FieldKind::Spinner { when: bld.expr(item.span(3, item.tokens.len()), item, ExprKind::Cond) }
            }
            other => {
                bld.err(item.err(format!("unknown field kind `{other}`")));
                continue;
            }
        };
        let rects = det.fields.iter().filter(|f| f.name == item.key()).map(|f| f.rect).collect();
        fields.push(FieldDef { name: item.key().to_string(), kind, align, rects });
    }
    Some(PanelDef { name: b.name.clone(), params, refresh, picture: det.picture, width: det.width, height: det.height, fields })
}

/// Parses an action from `item.tokens[from..to]`.
fn parse_action(bld: &mut Builder, item: &RawItem, from: usize, to: usize) -> Option<Action> {
    let t = &item.tokens[from..to];
    let Some(first) = t.first() else {
        bld.err(item.err("missing action"));
        return None;
    };
    let need_args = |bld: &mut Builder, n: usize| {
        if t.len() != n + 1 {
            bld.err(item.err(format!("`{}` takes {n} argument(s)", first.text)));
            false
        } else {
            true
        }
    };
    let action = match first.text.as_str() {
        "open" => {
            let (name, args) = match split_call(item.span(from + 1, to)) {
                Ok(x) => x,
                Err(e) => {
                    bld.err(item.err(e));
                    return None;
                }
            };
            if name.is_empty() || (args.is_none() && t.len() != 2) {
                bld.err(item.err("`open NAME` or `open NAME(arguments)`"));
                return None;
            }
            let target = bld.resolve(item, name, &["menu", "panel", "input", "layout"])?;
            let args = args.map(|a| parse_args(bld, item, a)).unwrap_or_default();
            check_args(bld, item, &target, &args);
            Action::Open { target, args }
        }
        "set" => {
            let var = t.get(1)?.text.clone();
            if !bld.vars.contains_key(&var) {
                bld.err(item.err(format!("unknown variable `{var}`")));
            }
            if t.len() < 3 {
                bld.err(item.err("`set VARIABLE VALUE`"));
                return None;
            }
            Action::Set { var, value: bld.expr(item.span(from + 2, to), item, ExprKind::Value) }
        }
        "toggle" => {
            if !need_args(bld, 1) {
                return None;
            }
            let name = t[1].text.clone();
            match bld.vars.get(&name) {
                Some(VarType::Bool) => Action::Toggle(name),
                Some(_) => {
                    bld.err(item.err(format!("`toggle` needs a bool variable or a panel, `{name}` is neither")));
                    return None;
                }
                None => Action::TogglePanel(bld.resolve(item, &name, &["panel"])?),
            }
        }
        "adjust" => {
            if t.len() != 3 && !(t.len() == 5 && t[3].is("pool")) {
                bld.err(item.err("`adjust VARIABLE a..b [pool VARIABLE]`"));
                return None;
            }
            let var = t[1].text.clone();
            let check_number = |bld: &mut Builder, v: &str| match bld.vars.get(v) {
                Some(VarType::Number { .. }) => {}
                Some(_) => bld.err(item.err(format!("`adjust` needs number variables, `{v}` is not one"))),
                None => bld.err(item.err(format!("unknown variable `{v}`"))),
            };
            check_number(bld, &var);
            let Some((lo, hi)) = parse_range(&t[2].text) else {
                bld.err(item.err(format!("bad range `{}`", t[2].text)));
                return None;
            };
            let pool = t.get(4).map(|p| p.text.clone());
            if let Some(p) = &pool {
                check_number(bld, p);
            }
            Action::Adjust { var, lo, hi, pool }
        }
        "emit" => {
            if !need_args(bld, 1) {
                return None;
            }
            Action::Emit(t[1].text.clone())
        }
        "stop" => {
            if !need_args(bld, 1) {
                return None;
            }
            Action::Stop(bld.resolve(item, &t[1].text, &["command"])?)
        }
        "focus" => {
            if !need_args(bld, 1) {
                return None;
            }
            Action::Focus(parse_focus(bld, item, &t[1].text)?)
        }
        "run" => {
            let mut end = to;
            let interactive = item.tokens[to - 1].is("interactive") && to - from > 2;
            if interactive {
                end -= 1;
            }
            if end - from < 2 {
                bld.err(item.err("`run COMMANDLINE`"));
                return None;
            }
            let toks = &item.tokens[from + 1..end];
            let cmdline = if toks.len() == 1 && toks[0].quoted { toks[0].text.clone() } else { item.span(from + 1, end).to_string() };
            Action::Run { cmdline, interactive }
        }
        word => {
            let word = action_alias(word);
            if let Some(a) = Action::builtin(word) {
                if !need_args(bld, 0) {
                    return None;
                }
                a
            } else if t.len() == 1 {
                Action::Command(bld.resolve(item, word, &["command"])?)
            } else {
                bld.err(item.err(format!("unknown action `{word}`")));
                return None;
            }
        }
    };
    Some(action)
}

fn build_command(bld: &mut Builder, b: &RawBlock) -> CommandDef {
    let mut c = CommandDef {
        name: b.name.clone(),
        confirm: None,
        run: None,
        interactive: false,
        env: vec![],
        output: None,
        then: None,
        on_error: None,
        into: None,
        secrets: vec![],
    };
    if !b.args.is_empty() {
        bld.err(b.err("command header takes no options"));
    }
    for item in &b.items {
        let one = |bld: &mut Builder| -> Option<String> {
            if item.tokens.len() == 2 {
                Some(item.tokens[1].text.clone())
            } else {
                bld.err(item.err(format!("`{}` takes one value", item.key())));
                None
            }
        };
        match item.key() {
            "confirm" => c.confirm = one(bld).map(|k| bld.text_key(item, &k)),
            "run" => {
                if let Some(Action::Run { cmdline, interactive }) = parse_action(bld, item, 0, item.tokens.len()) {
                    c.run = Some(cmdline);
                    c.interactive = interactive;
                }
            }
            "env" => {
                for t in &item.tokens[1..] {
                    match t.text.split_once('=') {
                        Some((k, v)) if is_var_name(k) && !t.quoted => {
                            let e = bld.expr(v, item, ExprKind::Value);
                            c.env.push((k.to_string(), e));
                        }
                        _ => bld.err(item.err(format!("env entries are NAME=VALUE, got `{}`", t.text))),
                    }
                }
            }
            "output" => {
                if let Some(o) = one(bld) {
                    c.output = bld.resolve(item, &o, &["log"]);
                }
            }
            "then" => c.then = parse_action(bld, item, 1, item.tokens.len()),
            "on_error" => c.on_error = parse_action(bld, item, 1, item.tokens.len()),
            "into" => {
                c.into = one(bld);
                if let Some(v) = &c.into
                    && !bld.vars.contains_key(v) {
                        bld.err(item.err(format!("unknown variable `{v}`")));
                    }
            }
            "secret" => match item.tokens.get(1..) {
                Some([v, m]) => {
                    let mode = match m.text.as_str() {
                        "stdin" => SecretMode::Stdin,
                        "env" => SecretMode::Env,
                        _ => {
                            bld.err(item.err("`secret VARIABLE stdin|env`"));
                            continue;
                        }
                    };
                    if !bld.vars.contains_key(&v.text) {
                        bld.err(item.err(format!("unknown variable `{}`", v.text)));
                    }
                    c.secrets.push((v.text.clone(), mode));
                }
                _ => bld.err(item.err("`secret VARIABLE stdin|env`")),
            },
            other => bld.err(item.err(format!("unknown field `{other}` in command block"))),
        }
    }
    if c.run.is_none() && c.then.is_none() {
        bld.err(b.err(format!("command `{}` needs `run` or `then`", c.name)));
    }
    if c.secrets.iter().filter(|(_, m)| *m == SecretMode::Stdin).count() > 1 {
        bld.err(b.err("only one secret can be passed on stdin"));
    }
    c
}

fn build_menu(bld: &mut Builder, b: &RawBlock) -> MenuDef {
    let mut context = None;
    let mut size = (None, None);
    let mut is_list = false;
    let mut final_ = false;
    let mut i = 0;
    while i < b.args.len() {
        match b.args[i].text.as_str() {
            "list" => is_list = true,
            "final" => final_ = true,
            w @ ("context" | "size") => {
                let Some(v) = b.args.get(i + 1) else {
                    bld.err(b.err(format!("`{w}` needs a value")));
                    break;
                };
                if w == "context" {
                    context = Some(v.text.clone());
                } else {
                    match Dim::parse_pair(&v.text) {
                        Ok(s) => size = s,
                        Err(e) => bld.err(b.err(e)),
                    }
                }
                i += 1;
            }
            other => bld.err(b.err(format!("unknown menu option `{other}` (list, size WxH, context NAME, final)"))),
        }
        i += 1;
    }
    let kind = if is_list { MenuKind::List(Box::new(build_list(bld, b))) } else { build_static_menu(bld, b) };
    if final_ {
        let leaves = match &kind {
            MenuKind::Static(items) => items
                .iter()
                .any(|it| it.hide_when.is_none() && it.disabled_when.is_none() && matches!(it.action, Action::Quit | Action::Open { .. })),
            MenuKind::List(_) => false,
        };
        if !leaves {
            bld.err(b.err(format!(
                "final menu `{}` needs an item that is never hidden or disabled and leaves the menu (quit or open)",
                b.name
            )));
        }
    }
    MenuDef { name: b.name.clone(), params: b.params.clone().unwrap_or_default(), kind, context, size, final_ }
}

/// Positions of `hide when` / `disabled when` in an item.
struct ItemTail {
    action_end: usize,
    hide: Option<(usize, usize)>,
    disabled: Option<(usize, usize)>,
}

fn item_tail(item: &RawItem, from: usize) -> ItemTail {
    let t = &item.tokens;
    let end = t.len();
    let kw = |name: &str| (from..end.saturating_sub(1)).find(|&i| t[i].is(name) && t[i + 1].is("when"));
    let h = kw("hide");
    let d = kw("disabled");
    let action_end = [h, d].iter().flatten().copied().min().unwrap_or(end);
    let range = |start: Option<usize>, other: Option<usize>| start.map(|s| (s + 2, other.filter(|o| *o > s).unwrap_or(end)));
    ItemTail { action_end, hide: range(h, d), disabled: range(d, h) }
}

fn build_static_menu(bld: &mut Builder, b: &RawBlock) -> MenuKind {
    let mut items: Vec<MenuItem> = Vec::new();
    for item in &b.items {
        let key = item.key().to_string();
        if RESERVED_ITEMS.contains(&key.as_str()) {
            bld.err(item.err(format!("`{key}` is reserved for the menu's own text key")));
            continue;
        }
        if !is_name(&key) {
            bld.err(item.err(format!("menu item `{key}` must be an UPPER_CASE text key")));
            continue;
        }
        if items.iter().any(|i| i.key == key) {
            bld.err(item.err(format!("duplicate menu item `{key}`")));
            continue;
        }
        let mut from = 1;
        if let Some(d) = item.tokens.get(1).filter(|t| t.quoted) {
            bld.texts.defaults.insert(format!("{}.{key}", b.name), d.text.clone());
            from = 2;
        }
        let tail = item_tail(item, from);
        let action = if tail.action_end > from {
            parse_action(bld, item, from, tail.action_end)
        } else {
            // No action: the command of the same name, else the built-in
            // action of the same name in lower case (D 11.2).
            match bld.try_resolve(item, &key, &["command"]) {
                Some(full) => Some(Action::Command(full)),
                None => match Action::builtin(&key.to_ascii_lowercase()) {
                    Some(a) => Some(a),
                    None => {
                        bld.err(item.err(format!(
                            "`{key}` has no action and there is no command `{key}` or action `{}`",
                            key.to_ascii_lowercase()
                        )));
                        None
                    }
                },
            }
        };
        let hide_when = tail.hide.map(|(s, e)| bld.expr(item.span(s, e), item, ExprKind::Cond));
        let disabled_when = tail.disabled.map(|(s, e)| bld.expr(item.span(s, e), item, ExprKind::Cond));
        if let Some(action) = action {
            items.push(MenuItem { key, action, hide_when, disabled_when });
        }
    }
    if b.items.is_empty() {
        bld.err(b.err(format!("menu `{}` has no items", b.name)));
    }
    MenuKind::Static(items)
}

fn build_list(bld: &mut Builder, b: &RawBlock) -> ListDef {
    let mut source = None;
    let mut l = ListDef {
        source: Expr::bare(""),
        row: vec![],
        sort: None,
        filter: false,
        multi: false,
        keep: false,
        then: None,
        hide_when: None,
        disabled_when: None,
        columns: vec![],
    };
    let mut row_item = None;
    for item in &b.items {
        let n = item.tokens.len();
        let flag = |bld: &mut Builder| {
            if n != 1 {
                bld.err(item.err(format!("`{}` takes no value", item.key())));
            }
            true
        };
        match item.key() {
            "source" => source = Some(bld.expr(item.rest(1), item, ExprKind::Source)),
            "row" => {
                l.row = parse_named(bld, item, 1, n);
                row_item = Some(item);
            }
            "sort" => {
                let desc = n > 2 && item.tokens[n - 1].is("desc");
                let end = if desc { n - 1 } else { n };
                l.sort = Some((bld.expr(item.span(1, end), item, ExprKind::Value), desc));
            }
            "filter" => l.filter = flag(bld),
            "multi" => l.multi = flag(bld),
            "keep" => l.keep = flag(bld),
            "then" => l.then = parse_action(bld, item, 1, n),
            "hide" | "disabled" => {
                if !item.tokens.get(1).is_some_and(|t| t.is("when")) {
                    bld.err(item.err(format!("`{} when CONDITION`", item.key())));
                    continue;
                }
                let e = bld.expr(item.rest(2), item, ExprKind::Cond);
                if item.key() == "hide" {
                    l.hide_when = Some(e);
                } else {
                    l.disabled_when = Some(e);
                }
            }
            "columns" => {
                let mut cols: Vec<Column> = Vec::new();
                for t in &item.tokens[1..] {
                    if let Some(a) = Align::parse(&t.text) {
                        match cols.last_mut() {
                            Some(c) => c.align = a,
                            None => bld.err(item.err("alignment must follow a column")),
                        }
                        continue;
                    }
                    let Some((name, w)) = t.text.split_once(':') else {
                        bld.err(item.err(format!("column `{}` must be NAME:WIDTH or NAME:*", t.text)));
                        continue;
                    };
                    let width = if w == "*" {
                        None
                    } else if let Ok(n) = w.parse() {
                        Some(n)
                    } else {
                        bld.err(item.err(format!("bad column width `{w}`")));
                        continue;
                    };
                    cols.push(Column { name: name.to_string(), width, align: Align::Left });
                }
                l.columns = cols;
            }
            other => bld.err(item.err(format!("unknown field `{other}` in list menu"))),
        }
    }
    match source {
        Some(s) => l.source = s,
        None => bld.err(b.err(format!("list menu `{}` needs `source`", b.name))),
    }
    if let Some(item) = row_item {
        for (k, _) in &l.row {
            if !l.columns.is_empty() && !l.columns.iter().any(|c| c.name == *k) {
                bld.err(item.err(format!("row value for unknown column `{k}`")));
            }
        }
    }
    l
}

fn build_input(bld: &mut Builder, b: &RawBlock) -> InputDef {
    let mut inp = InputDef { name: b.name.clone(), var: String::new(), kind: InputKind::Text, validate: None, error: None, then: None, clear: false };
    for item in &b.items {
        let one = |bld: &mut Builder| -> Option<String> {
            if item.tokens.len() == 2 {
                Some(item.tokens[1].text.clone())
            } else {
                bld.err(item.err(format!("`{}` takes one value", item.key())));
                None
            }
        };
        match item.key() {
            "var" => {
                if let Some(v) = one(bld) {
                    if !bld.vars.contains_key(&v) {
                        bld.err(item.err(format!("unknown variable `{v}`")));
                    }
                    inp.var = v;
                }
            }
            "kind" => {
                inp.kind = match one(bld).as_deref() {
                    Some("text") | None => InputKind::Text,
                    Some("number") => InputKind::Number,
                    Some("password") => InputKind::Password,
                    Some("path") => InputKind::Path,
                    Some("choice") => InputKind::Choice,
                    Some(o) => {
                        bld.err(item.err(format!("unknown input kind `{o}` (text, number, password, path, choice)")));
                        InputKind::Text
                    }
                }
            }
            "validate" => inp.validate = Some(bld.expr(item.rest(1), item, ExprKind::Cond)),
            "error" => inp.error = one(bld).map(|k| bld.text_key(item, &k)),
            "then" => inp.then = parse_action(bld, item, 1, item.tokens.len()),
            "clear" if item.tokens.len() == 1 => inp.clear = true,
            other => bld.err(item.err(format!("unknown field `{other}` in input block"))),
        }
    }
    if inp.var.is_empty() {
        bld.err(b.err(format!("input `{}` needs `var`", b.name)));
    }
    match (inp.kind, bld.vars.get(&inp.var)) {
        (InputKind::Choice, Some(VarType::Choice(_))) | (_, None) => {}
        (InputKind::Choice, Some(_)) => bld.err(b.err("a `choice` input needs a choice variable")),
        (InputKind::Number, Some(VarType::Number { .. })) => {}
        (InputKind::Number, Some(_)) => bld.err(b.err("a `number` input needs a number variable")),
        _ => {}
    }
    inp
}

fn build_log(bld: &mut Builder, b: &RawBlock) -> LogDef {
    let mut l = LogDef { name: b.name.clone(), history: 1000, merge: false, more: false, keep_ansi: false };
    let mut i = 0;
    while i < b.args.len() {
        match (b.args[i].text.as_str(), b.args.get(i + 1)) {
            ("history", Some(n)) => {
                match n.text.parse() {
                    Ok(n) => l.history = n,
                    Err(_) => bld.err(b.err(format!("bad history size `{}`", n.text))),
                }
                i += 1;
            }
            ("ansi", Some(m)) if m.text == "keep" || m.text == "strip" => {
                l.keep_ansi = m.text == "keep";
                i += 1;
            }
            ("more", _) => l.more = true,
            ("merge", _) => l.merge = true,
            (other, _) => bld.err(b.err(format!("unknown log option `{other}` (history N, merge, more, ansi keep|strip)"))),
        }
        i += 1;
    }
    if let Some(item) = b.items.first() {
        bld.err(item.err("a Termplate log takes no rows (RLML log rows select game messages)"));
    }
    l
}

fn build_keys(bld: &mut Builder, b: &RawBlock, keymap: &mut Keymap) {
    let ctx = b.name.as_str();
    let pad = b.kind == "pad";
    if !bld.contexts.contains(ctx) {
        let known: Vec<String> = bld.contexts.iter().cloned().collect();
        bld.err(b.err(format!("unknown key context `{ctx}` (known: {})", known.join(", "))));
    }
    let mut i = 0;
    while i < b.args.len() {
        match b.args[i].text.as_str() {
            "modal" if !pad => keymap.set_mode(ctx, Mode::Modal),
            "passthrough" if !pad => keymap.set_mode(ctx, Mode::Passthrough),
            "deadzone" if pad => {
                let ok = b
                    .args
                    .get(i + 1)
                    .and_then(|t| t.text.strip_suffix('%'))
                    .and_then(|p| p.parse::<u8>().ok())
                    .is_some_and(|p| p <= 100);
                if !ok {
                    bld.err(b.err("`deadzone P%`"));
                }
                i += 1;
            }
            w => bld.err(b.err(format!("unknown {} option `{w}`", b.kind))),
        }
        i += 1;
    }
    let mut seen: BTreeMap<Binding, String> = BTreeMap::new();
    let mut actions = BTreeSet::new();
    for item in &b.items {
        let t = &item.tokens;
        let (action, keys_from) = if t.len() >= 2 && t[0].is("focus") {
            let Some(n) = parse_focus(bld, item, &t[1].text) else { continue };
            (format!("focus {n}"), 2)
        } else if t.len() >= 2 && (t[0].is("open") || t[0].is("toggle")) {
            let name = match parse_action(bld, item, 0, 2) {
                Some(Action::Open { target, .. }) => format!("open {target}"),
                Some(Action::Toggle(v)) => format!("toggle {v}"),
                Some(Action::TogglePanel(p)) => format!("toggle {p}"),
                _ => continue,
            };
            (name, 2)
        } else {
            let word = action_alias(item.key());
            if KEY_ACTIONS.contains(&word) {
                (word.to_string(), 1)
            } else if word.starts_with(|c: char| c.is_ascii_uppercase()) {
                match bld.resolve(item, word, &["command"]) {
                    Some(full) => (full, 1),
                    None => continue,
                }
            } else {
                bld.err(item.err(format!("unknown action `{word}`")));
                continue;
            }
        };
        if !actions.insert(action.clone()) {
            bld.err(item.err(format!("duplicate action `{action}` in {} {ctx}", b.kind)));
        }
        if t.len() <= keys_from {
            bld.err(item.err(format!("`{action}` needs at least one {}", if pad { "button" } else { "key" })));
        }
        let mut bindings = Vec::new();
        for tok in &t[keys_from..] {
            let parsed = if pad { parse_pad(&tok.text) } else { parse_binding(&tok.text) };
            match parsed {
                Ok(bnd) => {
                    if let Some(other) = seen.insert(bnd.clone(), action.clone())
                        && other != action {
                            bld.err(item.err(format!("`{}` is bound to both `{other}` and `{action}` in {ctx}", tok.text)));
                        }
                    bindings.push(bnd);
                }
                Err(e) => bld.err(item.err(e)),
            }
        }
        if pad {
            // Gamepad rows add to the action's keyboard bindings.
            let mut existing: Vec<Binding> = keymap
                .contexts()
                .find(|(c, _)| *c == ctx)
                .and_then(|(_, l)| l.iter().find(|(a, _)| *a == action).map(|(_, b)| b.clone()))
                .unwrap_or_default();
            existing.retain(|b| !matches!(b, Binding::Pad(_)));
            existing.extend(bindings);
            keymap.bind(ctx, &action, existing);
        } else {
            keymap.bind(ctx, &action, bindings);
        }
    }
}

fn build_text(bld: &mut Builder, b: &RawBlock) {
    let lang = b.name.clone();
    for item in &b.items {
        let key = names::qualify(item.namespace.as_deref(), item.key());
        if !is_name(&key) {
            bld.err(item.err(format!("text key `{key}` must be UPPER_CASE")));
            continue;
        }
        let text = if item.tokens.len() == 2 && item.tokens[1].quoted { item.tokens[1].text.clone() } else { item.rest(1).to_string() };
        bld.texts.langs.entry(lang.clone()).or_default().insert(key.clone(), text);
        bld.text_locs.insert((lang.clone(), key), (item.file.clone(), item.line));
    }
}

fn build_colors(bld: &mut Builder, b: &RawBlock, style: &mut StyleSheet) {
    for item in &b.items {
        if !is_name(item.key()) {
            bld.err(item.err(format!("color name `{}` must be UPPER_CASE", item.key())));
            continue;
        }
        let name = names::qualify(item.namespace.as_deref(), item.key());
        match item.tokens.get(1).filter(|_| item.tokens.len() == 2).and_then(|t| crate::style::parse_hex(&t.text)) {
            Some(c) => {
                style.palette.insert(name, c);
            }
            None => bld.err(item.err("`- NAME hex` with six lower-case hex digits, e.g. `- STONE 5a5a6e`")),
        }
    }
}

fn build_style_rows(bld: &mut Builder, b: &RawBlock, style: &mut StyleSheet) {
    for item in &b.items {
        let raw = item.key();
        let (base, variant) = match raw.rsplit_once('.') {
            Some((base, v)) if VARIANTS.contains(&v) => (base, Some(v)),
            _ => (raw, None),
        };
        let target = if bld.contexts.contains(base) {
            Some(base.to_string())
        } else {
            bld.resolve(item, base, &["menu", "panel", "log", "input"])
        };
        let Some(target) = target else { continue };
        let words: Vec<&str> = item.tokens[1..].iter().map(|t| t.text.as_str()).collect();
        match style.parse_row(&words) {
            Ok(row) => {
                let key = match variant {
                    Some(v) => format!("{target}.{v}"),
                    None => target,
                };
                style.rows.insert(key, row);
            }
            Err(e) => bld.err(item.err(e)),
        }
    }
}

/// The area number of `focus N`: 1–9.
fn parse_focus(bld: &mut Builder, item: &RawItem, n: &str) -> Option<usize> {
    match n.parse::<usize>() {
        Ok(n @ 1..=9) => Some(n),
        _ => {
            bld.err(item.err(format!("`focus N` takes an area number 1-9, got `{n}`")));
            None
        }
    }
}

fn build_settings(bld: &mut Builder, b: &RawBlock) -> Settings {
    let mut s = Settings::default();
    for item in &b.items {
        let Some(v) = item.tokens.get(1).filter(|_| item.tokens.len() == 2).map(|t| t.text.clone()) else {
            bld.err(item.err(format!("`{}` takes one value", item.key())));
            continue;
        };
        match item.key() {
            "language" => s.language = Some(v),
            "theme" | "style" => s.theme = Some(v),
            "refresh" => match v.parse::<f64>() {
                Ok(f) if f > 0.0 => s.refresh_factor = Some(f),
                _ => bld.err(item.err("`refresh` is a positive factor, e.g. 2 or 0.5")),
            },
            "timeout" => match parse_duration(&v) {
                Some(d) => s.timeout = Some(d),
                None => bld.err(item.err(format!("bad duration `{v}`"))),
            },
            "mouse" => match v.as_str() {
                "on" => s.mouse = Some(true),
                "off" => s.mouse = Some(false),
                _ => bld.err(item.err("`mouse on|off`")),
            },
            "area_numbers" => match v.as_str() {
                "on" => s.area_numbers = true,
                "off" => s.area_numbers = false,
                _ => bld.err(item.err("`area_numbers on|off`")),
            },
            other => bld.err(item.err(format!("unknown setting `{other}`"))),
        }
    }
    s
}

/// `{@KEY}` must exist and must not form cycles, and `{^ACTION}` must name
/// an action or command (D 10, M 2.5).
fn check_texts(bld: &mut Builder, model: &Model) {
    let all_keys: BTreeSet<String> =
        bld.texts.langs.values().flat_map(|m| m.keys().cloned()).chain(bld.texts.defaults.keys().cloned()).collect();
    let mut problems: Vec<LoadError> = Vec::new();
    for (lang, map) in &bld.texts.langs {
        for (key, text) in map {
            let (file, line) = bld.text_locs.get(&(lang.clone(), key.clone())).cloned().unwrap_or_default();
            for ph in placeholders(text) {
                if let Some(r) = ph.strip_prefix('@') {
                    if !all_keys.contains(r) {
                        problems.push(LoadError::new(&file, line, format!("{key}: unknown text key `{{@{r}}}`")));
                    }
                } else if let Some(a) = ph.strip_prefix('^') {
                    let known = KEY_ACTIONS.contains(&action_alias(a))
                        || model.commands.contains_key(a)
                        || a.starts_with("open ")
                        || a.starts_with("toggle ")
                        || a.starts_with("focus ");
                    if !known {
                        problems.push(LoadError::new(&file, line, format!("{key}: unknown action or command `{{^{a}}}`")));
                    }
                }
            }
            let mut path = vec![key.clone()];
            if let Some(cycle) = find_cycle(map, key, &mut path) {
                let mut sorted = cycle.clone();
                sorted.sort();
                if sorted.first() == Some(key) {
                    problems.push(LoadError::new(&file, line, format!("text reference cycle: {}", cycle.join(" → "))));
                }
            }
        }
    }
    bld.errors.0.extend(problems);
}

/// The `{…}` placeholders of a text (without braces; `{{` is an escape).
pub fn placeholders(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = text;
    while let Some(i) = rest.find('{') {
        if rest[i + 1..].starts_with('{') {
            rest = &rest[i + 2..];
            continue;
        }
        let Some(j) = rest[i..].find('}') else { break };
        out.push(rest[i + 1..i + j].to_string());
        rest = &rest[i + j + 1..];
    }
    out
}

fn find_cycle(map: &BTreeMap<String, String>, key: &str, path: &mut Vec<String>) -> Option<Vec<String>> {
    let text = map.get(key)?;
    for ph in placeholders(text) {
        let Some(r) = ph.strip_prefix('@') else { continue };
        if path.first().is_some_and(|f| f == r) {
            let mut c = path.clone();
            c.push(r.to_string());
            return Some(c);
        }
        if path.iter().any(|p| p == r) || path.len() > 16 {
            continue;
        }
        path.push(r.to_string());
        if let Some(c) = find_cycle(map, r, path) {
            return Some(c);
        }
        path.pop();
    }
    None
}

#[cfg(test)]
mod tests;
