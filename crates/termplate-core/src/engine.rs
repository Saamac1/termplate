//! The runtime engine: screen stack, focus, menus, inputs, logs, panels,
//! command execution and drawing (spec §4–§10, §12, §14; RLML M 7, M 8).
//!
//! The engine is I/O free. Callers feed it [`InputEvent`]s, timer ticks and
//! coprocess messages, draw the [`Frame`] it renders and act on the
//! [`Effect`]s it produces. Expressions and commands go through [`Host`].

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::time::{Duration, Instant};

use crate::frame::{str_width, wrap, Frame, Role};
use crate::host::{eval_now, EvalRequest, EvalResults, Evaluated, Expr, ExprKind, Host, JobEvent, JobId, Outcome, RunRequest, Scope};
use crate::keys::{Binding, KeyCode, Mods, Mouse, StackEntry, KEY_ACTIONS};
use crate::layout::Rect;
use crate::menu::{row_letter, MenuState, Row};
use crate::model::{Action, Align, Arg, CommandDef, FieldKind, InputKind, MenuKind, Model, SecretMode};
use crate::protocol::{Event, HostMsg};
use crate::style::Border;
use crate::text::TextContext;
use crate::value::{format_number, Value, VarType, Vars};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// `termplate run`: the layout loop; everything happens via commands.
    App,
    /// `termplate menu|input|panel`: one widget, result to stdout.
    Widget,
    /// `termplate serve`: driven by a coprocess script.
    Serve,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Effect {
    Quit,
    /// Widget mode result: exit code (0 choice, 1 back, 2 quit) and output.
    Exit { code: i32, output: Option<String> },
    Event(Event),
}

#[derive(Debug, Clone, PartialEq)]
pub enum InputEvent {
    Key(KeyCode, Mods),
    Mouse { binding: Binding, x: usize, y: usize },
    /// The pointer moved (menus highlight the row under it, M 8.1).
    MouseMove { x: usize, y: usize },
    Resize(usize, usize),
}

impl InputEvent {
    pub fn key(code: KeyCode) -> Self {
        InputEvent::Key(code, Mods::default())
    }
    pub fn ch(c: char) -> Self {
        InputEvent::Key(KeyCode::Char(c), Mods::default())
    }
    pub fn click(x: usize, y: usize) -> Self {
        InputEvent::Mouse { binding: Binding::mouse(Mouse::Click), x, y }
    }
}

#[derive(Debug, Clone, PartialEq)]
enum Screen {
    Menu(String),
    Panel(String),
    Input(String),
    Confirm { command: String, it: Option<String> },
    Output { title: String, lines: Vec<(String, bool)>, scroll: usize },
}

#[derive(Debug, Clone, PartialEq, Default)]
struct InputState {
    buf: Vec<char>,
    cursor: usize,
    error: Option<String>,
    /// The previous key was a completing `tab` (spec §8).
    tabbed: bool,
}

#[derive(Debug, Clone, PartialEq)]
struct LogEntry {
    text: String,
    stderr: bool,
    count: usize,
}

#[derive(Debug, Clone, PartialEq, Default)]
struct LogState {
    entries: VecDeque<LogEntry>,
    /// Entries scrolled back from the newest.
    back: usize,
    /// `more` paging (D 11.4): first entry not yet shown, and the page.
    unread_from: Option<usize>,
    page: usize,
    /// Set while drawing: a page with `<LOG>.MORE` is on screen.
    paging: bool,
}

#[derive(Debug, Clone, PartialEq)]
enum Hit {
    MenuRow { menu: String, index: usize },
    Area(usize),
    Input(String),
    /// An input field of a form: the panel and the field's number.
    FormField { panel: String, index: usize },
    Modal,
}

const SPINNER: [char; 4] = ['|', '/', '-', '\\'];

/// The built-in list menu of path completions (spec §8).
pub const COMPLETE_MENU: &str = "TERMPLATE.COMPLETE";
/// At most this many completions are listed.
const MAX_COMPLETIONS: usize = 200;
/// The last row of a cut completion list.
const MORE_ROW: &str = "…";
const MAX_CHAIN: usize = 16;

/// Built-in texts; applications may override them in their texts.
pub const BUILTIN_TEXTS: &[(&str, &str)] = &[
    ("TERMPLATE.TOO_SMALL", "Terminal too small: {WIDTH}x{HEIGHT} needed"),
    ("TERMPLATE.CONFIRM_HINT", "{^confirm} OK  {^back} Cancel"),
    ("TERMPLATE.MORE", "--More--"),
    ("TERMPLATE.INVALID", "Invalid value"),
    ("TERMPLATE.TIMEOUT", "Command timed out"),
    ("TERMPLATE.FAILED", "Command failed with status {STATUS}"),
    ("TERMPLATE.EMPTY", "(empty)"),
    ("TERMPLATE.LOADING", "…"),
    ("TERMPLATE.FORM_HINT", "{^confirm} OK  {^back} Cancel  Tab next field"),
    ("TERMPLATE.OUTPUT", "Output"),
    ("TERMPLATE.RUNNING", "{NAME} is already running"),
    ("TERMPLATE.CANCELLED", "{NAME} cancelled"),
];

/// A command running in the background (spec §15).
#[derive(Debug, Clone)]
struct Job {
    id: JobId,
    /// The command's name, or the command line of a `run` action.
    name: String,
    /// The `command` block; `None` for a `run` action.
    def: Option<CommandDef>,
    it: Option<String>,
    /// The screen or area that started it (see [`Engine::origin`]).
    origin: Option<String>,
    /// Output that is not streamed to a log area, kept for the end.
    stdout: String,
    stderr: String,
    depth: usize,
    cancelled: bool,
}

pub struct Engine {
    pub model: Model,
    pub mode: Mode,
    vars: Vars,
    secret_vars: BTreeSet<String>,
    layout: Option<String>,
    focus: usize,
    screens: Vec<Screen>,
    menus: BTreeMap<String, MenuState>,
    params: BTreeMap<String, BTreeMap<String, String>>,
    inputs: BTreeMap<String, InputState>,
    logs: BTreeMap<String, LogState>,
    panel_values: BTreeMap<String, BTreeMap<String, String>>,
    panel_refreshed: BTreeMap<String, Instant>,
    hidden_panels: BTreeSet<String>,
    dirty: bool,
    size: (usize, usize),
    effects: Vec<Effect>,
    busy: bool,
    spin: usize,
    hits: Vec<(Rect, Hit)>,
    area_rects: BTreeMap<String, Rect>,
    now: Instant,
    chain_depth: usize,
    jobs: Vec<Job>,
    /// The input a completion list fills in.
    completing: Option<String>,
    /// Background evaluations: job → menu or panel.
    evals: BTreeMap<JobId, String>,
    /// Menus and panels being evaluated; true when they changed meanwhile.
    pending: BTreeMap<String, bool>,
    /// Menus and panels evaluated at least once.
    loaded: BTreeSet<String>,
    /// The field being edited in each form (spec §8).
    form_field: BTreeMap<String, usize>,
    /// A form's validation message.
    form_error: BTreeMap<String, String>,
}

fn complete_menu() -> crate::model::MenuDef {
    use crate::model::{ListDef, MenuDef};
    MenuDef {
        name: COMPLETE_MENU.to_string(),
        params: vec![crate::syntax::Param { name: "matches".into(), ty: None }],
        kind: MenuKind::List(Box::new(ListDef {
            source: crate::host::Expr::bare("$matches"),
            row: vec![],
            sort: None,
            filter: true,
            multi: false,
            keep: false,
            then: None,
            hide_when: None,
            disabled_when: None,
            columns: vec![],
        })),
        context: None,
        size: (None, None),
        final_: false,
    }
}

/// Scrolls a one-line input so the cursor (a char index) stays visible in
/// `w` columns. Returns the first shown char and the cursor's column.
fn input_view(text: &str, cursor: usize, w: usize) -> (usize, usize) {
    let chars: Vec<char> = text.chars().collect();
    let col = |a: usize, b: usize| chars[a..b].iter().map(|c| crate::frame::char_width(*c)).sum::<usize>();
    let mut start = 0;
    while start < cursor && col(start, cursor) >= w.max(1) {
        start += 1;
    }
    (start, col(start, cursor))
}

fn next_value(r: &mut impl Iterator<Item = Evaluated>) -> Value {
    match r.next() {
        Some(Evaluated::Value(v)) => v,
        Some(Evaluated::Cond(b)) => Value::Bool(b),
        Some(Evaluated::Source(items)) => Value::List(items.iter().map(Value::to_text).collect()),
        None => Value::Text(String::new()),
    }
}

fn next_text(r: &mut impl Iterator<Item = Evaluated>) -> String {
    next_value(r).to_text()
}

fn next_cond(r: &mut impl Iterator<Item = Evaluated>) -> bool {
    match r.next() {
        Some(Evaluated::Cond(b)) => b,
        Some(Evaluated::Value(v)) => v.truthy(),
        _ => false,
    }
}

fn common_prefix(items: &[String]) -> String {
    let Some(first) = items.first() else { return String::new() };
    let mut len = first.chars().count();
    for it in &items[1..] {
        len = len.min(first.chars().zip(it.chars()).take_while(|(a, b)| a == b).count());
    }
    first.chars().take(len).collect()
}

struct Ctx<'a> {
    vars: &'a Vars,
    model: &'a Model,
    stack: Vec<StackEntry>,
    extra: &'a BTreeMap<String, String>,
}

impl TextContext for Ctx<'_> {
    fn var(&self, name: &str) -> Option<String> {
        self.extra.get(name).cloned().or_else(|| self.vars.get(name).map(Value::to_text))
    }
    fn key_for(&self, action: &str) -> Option<String> {
        let action = crate::keys::action_alias(action);
        let b = self.model.keymap.binding_for(&self.stack, action).or_else(|| self.model.keymap.any_binding_for(action));
        // An unbound action shows as its name (M 7.3).
        Some(b.map(|b| b.to_string()).unwrap_or_else(|| crate::keys::canonical_action(action).to_string()))
    }
}

impl Engine {
    pub fn new(mut model: Model, mode: Mode) -> Self {
        for (k, v) in BUILTIN_TEXTS {
            model.texts.defaults.entry(k.to_string()).or_insert(v.to_string());
        }
        let vars = model.vars.iter().map(|v| (v.name.clone(), v.default.clone())).collect();
        model.menus.insert(COMPLETE_MENU.to_string(), complete_menu());
        let secret_vars = model.inputs.values().filter(|i| i.kind == InputKind::Password).map(|i| i.var.clone()).collect();
        Engine {
            model,
            mode,
            vars,
            secret_vars,
            layout: None,
            focus: 0,
            screens: Vec::new(),
            menus: BTreeMap::new(),
            params: BTreeMap::new(),
            inputs: BTreeMap::new(),
            logs: BTreeMap::new(),
            panel_values: BTreeMap::new(),
            panel_refreshed: BTreeMap::new(),
            hidden_panels: BTreeSet::new(),
            dirty: true,
            size: (80, 24),
            effects: Vec::new(),
            busy: false,
            spin: 0,
            hits: Vec::new(),
            area_rects: BTreeMap::new(),
            now: Instant::now(),
            chain_depth: 0,
            jobs: Vec::new(),
            completing: None,
            evals: BTreeMap::new(),
            pending: BTreeMap::new(),
            loaded: BTreeSet::new(),
            form_field: BTreeMap::new(),
            form_error: BTreeMap::new(),
        }
    }

    // ----- public API -----------------------------------------------------

    /// Starts the application layout (the app block's `layout`).
    pub fn start_app(&mut self, host: &mut dyn Host) -> Result<(), String> {
        let layout = self.model.app.layout.clone().ok_or("the app has no layout")?;
        self.layout = Some(layout);
        // Load warnings are easy to miss on stderr behind the UI (spec §5).
        if let Some(area) = self.model.logs.keys().next().cloned() {
            for w in self.model.warnings.clone() {
                self.log(&area, &format!("warning: {w}"), true);
            }
        }
        self.focus = self.focusables().first().copied().unwrap_or(0);
        self.dirty = true;
        self.update(host);
        Ok(())
    }

    /// Starts with a single widget (a menu, input or panel) as the root.
    /// A list menu with no rows and no `<MENU>.EMPTY` text is not opened
    /// (D 11.2); the widget then ends with status 1.
    pub fn start_widget(&mut self, host: &mut dyn Host, target: &str) -> Result<(), String> {
        if !self.open(host, target, BTreeMap::new())? {
            self.effects.push(Effect::Exit { code: 1, output: None });
        }
        Ok(())
    }

    pub fn set_size(&mut self, w: usize, h: usize) {
        self.size = (w, h);
    }

    pub fn take_effects(&mut self) -> Vec<Effect> {
        std::mem::take(&mut self.effects)
    }

    pub fn vars(&self) -> &Vars {
        &self.vars
    }

    pub fn var(&self, name: &str) -> Option<&Value> {
        self.vars.get(name)
    }

    /// Sets a variable from text, parsing it with the variable's type.
    pub fn set_var_text(&mut self, name: &str, text: &str) -> Result<(), String> {
        let def = self.model.var(name).ok_or_else(|| format!("unknown variable `{name}`"))?;
        let v = def.ty.parse_value(text)?;
        self.set_var(name, v);
        Ok(())
    }

    pub fn set_var(&mut self, name: &str, v: Value) {
        self.vars.insert(name.to_string(), v);
        self.dirty = true;
        // An input in a layout area shows the new value (spec §8).
        let stale: Vec<String> = self
            .model
            .inputs
            .values()
            .filter(|i| i.var == name && !self.screens.iter().any(|s| matches!(s, Screen::Input(n) if *n == i.name)))
            .map(|i| i.name.clone())
            .collect();
        for i in stale {
            self.inputs.remove(&i);
        }
    }

    /// The edit buffer of an input, started from its variable.
    fn input_state(&mut self, name: &str) -> &mut InputState {
        if !self.inputs.contains_key(name) {
            let def = self.model.inputs.get(name);
            let current = match def {
                Some(d) if d.kind != InputKind::Password => self.vars.get(&d.var).map(Value::to_text).unwrap_or_default(),
                _ => String::new(),
            };
            let buf: Vec<char> = current.chars().collect();
            self.inputs.insert(name.to_string(), InputState { cursor: buf.len(), buf, error: None, tabbed: false });
        }
        self.inputs.get_mut(name).expect("inserted above")
    }

    /// Top screen name, for tests and diagnostics.
    pub fn top(&self) -> Option<String> {
        self.screens.last().map(|s| match s {
            Screen::Menu(n) | Screen::Input(n) | Screen::Panel(n) => n.clone(),
            Screen::Confirm { command, .. } => format!("confirm:{command}"),
            Screen::Output { .. } => "output".into(),
        })
    }

    pub fn menu_state(&self, name: &str) -> Option<&MenuState> {
        self.menus.get(name)
    }

    pub fn log_lines(&self, name: &str) -> Vec<String> {
        self.logs.get(name).map(|l| l.entries.iter().map(|e| e.text.clone()).collect()).unwrap_or_default()
    }

    pub fn focused_target(&self) -> Option<String> {
        self.layout_areas().get(self.focus).map(|(_, t)| t.clone())
    }

    pub fn panel_hidden(&self, name: &str) -> bool {
        self.hidden_panels.contains(name)
    }

    /// Appends text to a log area (`log` messages, command output). With
    /// `merge`, a message equal to the previous one only counts up.
    pub fn log(&mut self, area: &str, text: &str, stderr: bool) {
        let Some(def) = self.model.logs.get(area) else { return };
        let (history, merge, more, keep_ansi) = (def.history, def.merge, def.more, def.keep_ansi);
        let log = self.logs.entry(area.to_string()).or_default();
        for line in text.lines() {
            let line = if keep_ansi { line.to_string() } else { strip_ansi(line) };
            if merge
                && let Some(last) = log.entries.back_mut().filter(|e| e.text == line && e.stderr == stderr) {
                    last.count += 1;
                    continue;
                }
            if more && log.unread_from.is_none() {
                log.unread_from = Some(log.entries.len());
                log.page = 0;
            }
            log.entries.push_back(LogEntry { text: line, stderr, count: 1 });
        }
        while log.entries.len() > history {
            log.entries.pop_front();
            log.unread_from = log.unread_from.map(|u| u.saturating_sub(1));
        }
        log.back = 0;
    }

    pub fn handle(&mut self, host: &mut dyn Host, ev: InputEvent) {
        match ev {
            InputEvent::Resize(w, h) => self.set_size(w, h),
            InputEvent::Key(code, mods) => self.handle_key(host, code, mods),
            InputEvent::Mouse { binding, x, y } => self.handle_mouse(host, binding, x, y),
            InputEvent::MouseMove { x, y } => self.hover(x, y),
        }
        self.update(host);
    }

    /// Timer tick: refreshes panels whose `refresh` interval has passed and
    /// advances spinners.
    pub fn tick(&mut self, host: &mut dyn Host, now: Instant) {
        self.now = now;
        self.spin = self.spin.wrapping_add(1);
        let factor = self.model.settings.refresh_factor.unwrap_or(1.0);
        let due: Vec<String> = self
            .visible_panels()
            .into_iter()
            .filter(|p| {
                let Some(every) = self.model.panels.get(p).and_then(|d| d.refresh) else { return false };
                if self.pending.contains_key(p) {
                    return false;
                }
                self.panel_refreshed.get(p).is_none_or(|t| now.duration_since(*t) >= every.mul_f64(factor))
            })
            .collect();
        for p in due {
            self.eval_panel(host, &p);
        }
        self.update(host);
    }

    /// Time until the next panel refresh is due (for the event loop).
    pub fn next_refresh(&self) -> Option<Duration> {
        let factor = self.model.settings.refresh_factor.unwrap_or(1.0);
        self.visible_panels()
            .iter()
            .filter(|p| !self.pending.contains_key(*p))
            .filter_map(|p| {
                let every = self.model.panels.get(p)?.refresh?.mul_f64(factor);
                let last = self.panel_refreshed.get(p)?;
                Some(every.saturating_sub(self.now.duration_since(*last)))
            })
            .min()
    }

    /// Applies a coprocess message (spec §14.3).
    pub fn apply(&mut self, host: &mut dyn Host, msg: HostMsg) -> Result<(), String> {
        let r = match msg {
            HostMsg::Set { name, value } => self.set_var_text(&name, &value),
            HostMsg::Open(name) => self.open(host, &name, BTreeMap::new()).map(|_| ()),
            HostMsg::Close(None) => {
                self.close_top(true);
                Ok(())
            }
            HostMsg::Close(Some(name)) => {
                let before = self.screens.len();
                self.screens.retain(|s| !matches!(s, Screen::Menu(n) | Screen::Input(n) | Screen::Panel(n) if *n == name));
                if before == self.screens.len() { Err(format!("`{name}` is not open")) } else { Ok(()) }
            }
            HostMsg::Log(text) => {
                let (first, rest) = text.split_once(' ').unwrap_or((&text, ""));
                if self.model.logs.contains_key(first) {
                    self.log(first, rest, false);
                    Ok(())
                } else if let Some(area) = self.model.logs.keys().next().cloned() {
                    self.log(&area, &text, false);
                    Ok(())
                } else {
                    Err("no log area".into())
                }
            }
            HostMsg::Refresh => {
                self.refresh_all(host);
                Ok(())
            }
            HostMsg::Busy(b) => {
                self.busy = b;
                Ok(())
            }
            HostMsg::Quit => {
                self.effects.push(Effect::Quit);
                Ok(())
            }
        };
        self.update(host);
        r
    }

    pub fn busy(&self) -> bool {
        self.busy || !self.jobs.is_empty()
    }

    /// True while background commands run; the event loop then polls
    /// [`Engine::poll_jobs`] often.
    pub fn has_jobs(&self) -> bool {
        !self.jobs.is_empty() || !self.evals.is_empty()
    }

    /// Takes output and completions of background commands from the host:
    /// output goes to the command's log area as it comes, and a finished
    /// command continues its chain (`into`, `then`, `on_error`).
    pub fn poll_jobs(&mut self, host: &mut dyn Host) {
        let events = host.poll_jobs();
        if events.is_empty() {
            return;
        }
        let mut ended = false;
        for ev in events {
            match ev {
                JobEvent::Line { job, text, stderr } => {
                    let Some(j) = self.jobs.iter_mut().find(|j| j.id == job) else { continue };
                    let area = j.def.as_ref().and_then(|d| d.output.clone());
                    let into = j.def.as_ref().is_some_and(|d| d.into.is_some());
                    match area {
                        Some(area) if stderr || !into => self.log(&area, &text, stderr),
                        _ => {
                            let buf = if stderr { &mut j.stderr } else { &mut j.stdout };
                            buf.push_str(&text);
                            buf.push('\n');
                        }
                    }
                }
                JobEvent::Evaluated { job, results } => {
                    let Some(name) = self.evals.remove(&job) else { continue };
                    let stale = self.pending.remove(&name).unwrap_or(false);
                    self.apply_results(&name, results);
                    if stale {
                        self.evaluate(host, &name, false);
                    }
                }
                JobEvent::Done { job, outcome } => {
                    ended = true;
                    let Some(pos) = self.jobs.iter().position(|j| j.id == job) else { continue };
                    let j = self.jobs.remove(pos);
                    let out = Outcome { stdout: j.stdout, stderr: j.stderr, ..outcome };
                    let area = j.def.as_ref().and_then(|d| d.output.clone());
                    if j.cancelled {
                        // A cancelled command does not continue its chain.
                        let mut extra = BTreeMap::new();
                        extra.insert("NAME".to_string(), j.name.clone());
                        let msg = self.text("TERMPLATE.CANCELLED", &extra);
                        let stderr = format!("{}{msg}\n", out.stderr);
                        self.show_output(area.as_deref(), &j.name, &out.stdout, &stderr, false);
                        continue;
                    }
                    self.chain_depth = j.depth;
                    match &j.def {
                        Some(def) => self.finish_command(host, &j.name, def, j.it, out),
                        None => self.show_output(None, &j.name, &out.stdout, &out.stderr, out.timed_out),
                    }
                    self.chain_depth = 0;
                }
            }
        }
        // A finished command may have changed what conditions and sources
        // see; evaluation results alone do not start another round.
        if ended {
            self.dirty = true;
        }
        self.update(host);
    }

    /// Stops every background command (the program is ending).
    pub fn cancel_all(&mut self, host: &mut dyn Host) {
        for j in &mut self.jobs {
            j.cancelled = true;
            host.cancel(j.id);
        }
        for id in std::mem::take(&mut self.evals).into_keys() {
            host.cancel(id);
        }
        self.pending.clear();
    }

    /// The screen or layout area commands are started from: the top modal,
    /// or the focused area.
    fn origin(&self) -> Option<String> {
        match self.screens.last() {
            Some(Screen::Menu(n) | Screen::Input(n) | Screen::Panel(n)) => Some(n.clone()),
            Some(_) => None,
            None => self.focused_target(),
        }
    }

    /// `back` first cancels the newest command started from the current
    /// context; on the layout, where `back` has nothing to close, the
    /// newest command (spec §15). Returns true when it cancelled one.
    fn cancel_job(&mut self, host: &mut dyn Host) -> bool {
        let origin = self.origin();
        let running = |j: &Job| !j.cancelled;
        let idx = self
            .jobs
            .iter()
            .rposition(|j| running(j) && j.origin == origin)
            .or_else(|| if self.screens.is_empty() { self.jobs.iter().rposition(running) } else { None });
        let Some(i) = idx else { return false };
        self.jobs[i].cancelled = true;
        host.cancel(self.jobs[i].id);
        true
    }

    /// `back`, after cancelling a running command if there is one.
    fn back_or_cancel(&mut self, host: &mut dyn Host) {
        if !self.cancel_job(host) {
            self.nav_back();
        }
    }

    /// Refuses to start a command that is still running (spec §15).
    fn already_running(&mut self, name: &str) -> bool {
        if !self.jobs.iter().any(|j| j.name == name) {
            return false;
        }
        let mut extra = BTreeMap::new();
        extra.insert("NAME".to_string(), name.to_string());
        let msg = self.text("TERMPLATE.RUNNING", &extra);
        self.report(&msg);
        true
    }

    // ----- state helpers --------------------------------------------------

    /// Variables without secrets, for expression evaluation and commands.
    fn public_vars(&self) -> Vars {
        self.vars.iter().filter(|(k, _)| !self.secret_vars.contains(*k)).map(|(k, v)| (k.clone(), v.clone())).collect()
    }

    fn layout_areas(&self) -> Vec<(Rect, String)> {
        let Some(base) = &self.layout else { return vec![] };
        let (w, h) = self.size;
        let Some(def) = self.model.choose_layout(base, w, h) else { return vec![] };
        let solved = def.grid.solve(&self.model.area_sizes(def, w, h), w, h);
        let mut areas: Vec<(Rect, String)> =
            def.areas.iter().map(|(c, a)| (solved.rects.get(c).copied().unwrap_or_default(), a.target.clone())).collect();
        areas.sort_by_key(|(r, _)| (r.y, r.x));
        areas
    }

    fn area_args(&self, target: &str) -> Option<Vec<Arg>> {
        let base = self.layout.as_ref()?;
        let (w, h) = self.size;
        let def = self.model.choose_layout(base, w, h)?;
        def.areas.values().find(|a| a.target == target).map(|a| a.args.clone())
    }

    fn is_focusable(&self, target: &str) -> bool {
        self.model.menus.contains_key(target)
            || self.model.logs.contains_key(target)
            || self.model.inputs.contains_key(target)
            || self.model.panels.get(target).is_some_and(|p| p.fields.iter().any(|f| matches!(f.kind, FieldKind::Input { .. })))
    }

    fn focusables(&self) -> Vec<usize> {
        self.layout_areas().iter().enumerate().filter(|(_, (_, t))| self.is_focusable(t)).map(|(i, _)| i).collect()
    }

    fn visible_panels(&self) -> Vec<String> {
        let mut out: Vec<String> = self
            .layout_areas()
            .into_iter()
            .map(|(_, t)| t)
            .filter(|t| self.model.panels.contains_key(t) && !self.hidden_panels.contains(t))
            .collect();
        for s in &self.screens {
            if let Screen::Panel(name) = s {
                out.push(name.clone());
            }
        }
        out
    }

    fn open_menus(&self) -> Vec<String> {
        let mut out: Vec<String> =
            self.layout_areas().into_iter().map(|(_, t)| t).filter(|t| self.model.menus.contains_key(t)).collect();
        for s in &self.screens {
            if let Screen::Menu(n) = s {
                out.push(n.clone());
            }
        }
        out
    }

    /// The active widget: the top modal, else the focused layout area.
    fn active(&self) -> Option<Screen> {
        if let Some(s) = self.screens.last() {
            return Some(s.clone());
        }
        let target = self.focused_target()?;
        if self.model.menus.contains_key(&target) {
            Some(Screen::Menu(target))
        } else if self.model.inputs.contains_key(&target) {
            Some(Screen::Input(target))
        } else if self.model.panels.contains_key(&target) {
            Some(Screen::Panel(target))
        } else if self.model.logs.contains_key(&target) {
            Some(Screen::Output { title: target, lines: vec![], scroll: 0 })
        } else {
            None
        }
    }

    fn screen_context(&self, s: &Screen) -> String {
        match s {
            Screen::Menu(n) => self.model.menus.get(n).map(|m| m.context().to_string()).unwrap_or_else(|| "MENU".into()),
            Screen::Input(_) => "INPUT".into(),
            // A modal panel, the confirmation and output dialogs use the
            // `MENU` bindings (D 11.5).
            // A form takes the `INPUT` keys (`confirm`, `back`).
            Screen::Panel(n) if self.model.panels.get(n).is_some_and(|p| p.is_form()) => "INPUT".into(),
            Screen::Panel(_) | Screen::Confirm { .. } => "MENU".into(),
            Screen::Output { title, .. } if self.model.logs.contains_key(title) => "LOG".into(),
            Screen::Output { .. } => "MENU".into(),
        }
    }

    /// Key contexts, bottom first (spec §9, M 8): the base context, the
    /// focused area's context (passthrough, so layout keys keep working),
    /// then every open modal screen.
    pub fn context_stack(&self) -> Vec<StackEntry> {
        let mut stack = Vec::new();
        if self.layout.is_some() {
            if let Some(base) = &self.model.app.keys {
                stack.push(StackEntry::modal(base));
            }
            if let Some(t) = self.focused_target() {
                let area = if self.model.menus.contains_key(&t) {
                    Some(Screen::Menu(t))
                } else if self.model.logs.contains_key(&t) {
                    Some(Screen::Output { title: t, lines: vec![], scroll: 0 })
                } else if self.model.inputs.contains_key(&t) {
                    Some(Screen::Input(t))
                } else if self.model.panels.get(&t).is_some_and(|p| p.is_form()) {
                    Some(Screen::Panel(t))
                } else {
                    None
                };
                if let Some(a) = area {
                    stack.push(StackEntry::passthrough(&self.screen_context(&a)));
                }
            }
        }
        for s in &self.screens {
            stack.push(StackEntry::modal(&self.screen_context(s)));
        }
        stack
    }

    /// Re-evaluates menus and panels after a variable change.
    fn update(&mut self, host: &mut dyn Host) {
        let mut guard = 0;
        while self.dirty && guard < 4 {
            self.dirty = false;
            guard += 1;
            self.refresh_all(host);
        }
    }

    fn refresh_all(&mut self, host: &mut dyn Host) {
        for m in self.open_menus() {
            self.eval_menu(host, &m);
        }
        for p in self.visible_panels() {
            self.eval_panel(host, &p);
        }
    }

    fn bind_args(&self, host: &mut dyn Host, target: &str, args: &[Arg], it: Option<String>) -> BTreeMap<String, String> {
        let names: Vec<String> = self
            .model
            .panels
            .get(target)
            .map(|p| p.params.iter().map(|x| x.name.clone()).collect())
            .or_else(|| self.model.menus.get(target).map(|m| m.params.iter().map(|x| x.name.clone()).collect()))
            .unwrap_or_default();
        let vars = self.public_vars();
        let mut scope = Scope::new(&vars);
        scope.it = it;
        let mut out = BTreeMap::new();
        for (i, (name, e)) in args.iter().enumerate() {
            let key = name.clone().or_else(|| names.get(i).cloned()).unwrap_or_else(|| (i + 1).to_string());
            out.insert(key, host.eval_value(e, &scope).to_text());
        }
        out
    }

    /// Evaluates a menu or panel: at once when `now` (opening it) or when
    /// the host cannot do it in the background, else in the background,
    /// keeping the old values meanwhile (spec §5, §7). A target whose
    /// evaluation is still running is not started again, but evaluated
    /// once more when the running one ends.
    fn evaluate(&mut self, host: &mut dyn Host, name: &str, now: bool) {
        if let Some(stale) = self.pending.get_mut(name) {
            *stale = true;
            return;
        }
        let is_panel = self.model.panels.contains_key(name);
        if is_panel {
            self.bind_area_args(host, name);
        }
        let req = if is_panel { self.panel_request(name) } else { self.menu_request(name) };
        let Some(req) = req else { return };
        let vars = self.public_vars();
        let params = self.params.get(name).cloned().unwrap_or_default();
        let scope = Scope::new(&vars).with_params(&params);
        if !now && let Some(id) = host.eval_async(&req, &scope) {
            self.evals.insert(id, name.to_string());
            self.pending.insert(name.to_string(), false);
            return;
        }
        let results = eval_now(host, &req, &scope);
        self.apply_results(name, results);
    }

    fn apply_results(&mut self, name: &str, results: EvalResults) {
        if self.model.panels.contains_key(name) {
            self.apply_panel(name, results);
        } else {
            self.apply_menu(name, results);
        }
        self.loaded.insert(name.to_string());
    }

    fn eval_menu(&mut self, host: &mut dyn Host, name: &str) {
        self.evaluate(host, name, false);
    }

    /// The expressions of a menu: item conditions, or a list's source and
    /// row expressions.
    fn menu_request(&self, name: &str) -> Option<EvalRequest> {
        let def = self.model.menus.get(name)?;
        Some(match &def.kind {
            MenuKind::Static(items) => {
                let mut v = Vec::new();
                for it in items {
                    v.extend(it.hide_when.iter().chain(&it.disabled_when).map(|e| (e.clone(), ExprKind::Cond)));
                }
                EvalRequest::Batch(v)
            }
            MenuKind::List(l) => {
                let mut each: Vec<(Expr, ExprKind)> = l.row.iter().map(|(_, e)| (e.clone(), ExprKind::Value)).collect();
                each.extend(l.sort.iter().map(|(e, _)| (e.clone(), ExprKind::Value)));
                each.extend(l.hide_when.iter().chain(&l.disabled_when).map(|e| (e.clone(), ExprKind::Cond)));
                EvalRequest::Rows { source: l.source.clone(), each }
            }
        })
    }

    /// Builds a menu's rows from the results of [`Engine::menu_request`].
    fn apply_menu(&mut self, name: &str, results: EvalResults) {
        let Some(def) = self.model.menus.get(name).cloned() else { return };
        let params = self.params.get(name).cloned().unwrap_or_default();
        let rows = match (&def.kind, results) {
            (MenuKind::Static(items), EvalResults::Batch(r)) => {
                let mut r = r.into_iter();
                let mut rows = Vec::new();
                for it in items {
                    let hidden = it.hide_when.is_some() && next_cond(&mut r);
                    let disabled = it.disabled_when.is_some() && next_cond(&mut r);
                    if hidden {
                        continue;
                    }
                    let label = self.text(&format!("{name}.{}", it.key), &params);
                    rows.push(Row { key: it.key.clone(), label, columns: vec![], disabled });
                }
                rows
            }
            (MenuKind::List(l), EvalResults::Rows(items)) => {
                let row_key = format!("{name}.ROW");
                let mut rows: Vec<(Row, String)> = Vec::new();
                for (it, r) in items {
                    let mut r = r.into_iter();
                    let values: Vec<(String, String)> = l.row.iter().map(|(n, _)| (n.clone(), next_text(&mut r))).collect();
                    let sort_key = if l.sort.is_some() { next_text(&mut r) } else { String::new() };
                    let hidden = l.hide_when.is_some() && next_cond(&mut r);
                    let disabled = l.disabled_when.is_some() && next_cond(&mut r);
                    if hidden {
                        continue;
                    }
                    let columns: Vec<String> = l
                        .columns
                        .iter()
                        .map(|c| values.iter().find(|(n, _)| *n == c.name).map(|(_, v)| v.clone()).unwrap_or_default())
                        .collect();
                    // The row text is `<MENU>.ROW` with the row values as
                    // parameters (D 11.2); without that text the values
                    // (or the source value) are shown as they are.
                    let label = if self.model.texts.has(&row_key) {
                        let extra: BTreeMap<String, String> = values.iter().cloned().collect();
                        self.text(&row_key, &extra)
                    } else if values.is_empty() {
                        it.clone()
                    } else {
                        values.iter().map(|(_, v)| v.as_str()).collect::<Vec<_>>().join("  ")
                    };
                    rows.push((Row { key: it, label, columns, disabled }, sort_key));
                }
                if let Some((_, desc)) = &l.sort {
                    rows.sort_by(|a, b| compare_values(&a.1, &b.1));
                    if *desc {
                        rows.reverse();
                    }
                }
                rows.into_iter().map(|(r, _)| r).collect()
            }
            _ => return,
        };
        self.menus.entry(name.to_string()).or_default().set_rows(rows);
    }

    fn eval_panel(&mut self, host: &mut dyn Host, name: &str) {
        self.evaluate(host, name, false);
    }

    /// Layout areas compute their arguments with the panel; a modal panel
    /// keeps the arguments it was opened with (M 7.7).
    fn bind_area_args(&mut self, host: &mut dyn Host, name: &str) {
        if !self.screens.iter().any(|s| matches!(s, Screen::Panel(n) if n == name))
            && let Some(args) = self.area_args(name) {
                let bound = self.bind_args(host, name, &args, None);
                self.params.insert(name.to_string(), bound);
            }
    }

    /// The expressions of a panel's fields, in field order.
    fn panel_request(&self, name: &str) -> Option<EvalRequest> {
        let def = self.model.panels.get(name)?;
        let mut v = Vec::new();
        for f in &def.fields {
            match &f.kind {
                FieldKind::Text { params, .. } => v.extend(params.iter().map(|(_, e)| (e.clone(), ExprKind::Value))),
                FieldKind::Value { expr } => v.push((expr.clone(), ExprKind::Value)),
                FieldKind::Bar { value, max } => {
                    v.push((value.clone(), ExprKind::Value));
                    v.push((max.clone(), ExprKind::Value));
                }
                FieldKind::Spinner { when } => v.push((when.clone(), ExprKind::Cond)),
                FieldKind::Param { .. } | FieldKind::Input { .. } => {}
            }
        }
        Some(EvalRequest::Batch(v))
    }

    /// Field values from the results of [`Engine::panel_request`].
    fn apply_panel(&mut self, name: &str, results: EvalResults) {
        let Some(def) = self.model.panels.get(name).cloned() else { return };
        let EvalResults::Batch(r) = results else { return };
        let mut r = r.into_iter();
        let params = self.params.get(name).cloned().unwrap_or_default();
        let mut values = BTreeMap::new();
        for f in &def.fields {
            let v = match &f.kind {
                FieldKind::Text { key, params: tp } => {
                    let mut extra = params.clone();
                    for (n, _) in tp {
                        extra.insert(n.clone(), next_text(&mut r));
                    }
                    self.text(key, &extra)
                }
                FieldKind::Value { .. } => next_text(&mut r),
                FieldKind::Param { name } => params.get(name).cloned().unwrap_or_default(),
                FieldKind::Bar { .. } => {
                    let v = next_value(&mut r).as_number().unwrap_or(0.0);
                    let m = next_value(&mut r).as_number().unwrap_or(0.0);
                    format!("{} {}", format_number(v), format_number(m))
                }
                FieldKind::Input { input } => {
                    let var = self.model.inputs.get(input).map(|i| i.var.clone()).unwrap_or_default();
                    self.vars.get(&var).map(Value::to_text).unwrap_or_default()
                }
                FieldKind::Spinner { .. } => next_cond(&mut r).to_string(),
            };
            values.insert(f.name.clone(), v);
        }
        self.panel_values.insert(name.to_string(), values);
        self.panel_refreshed.insert(name.to_string(), self.now);
    }

    fn text(&self, key: &str, extra: &BTreeMap<String, String>) -> String {
        let ctx = Ctx { vars: &self.vars, model: &self.model, stack: self.context_stack(), extra };
        self.model.texts.get(key, &ctx)
    }

    fn text_opt(&self, key: &str) -> Option<String> {
        let extra = BTreeMap::new();
        let ctx = Ctx { vars: &self.vars, model: &self.model, stack: self.context_stack(), extra: &extra };
        self.model.texts.get_opt(key, &ctx)
    }

    // ----- opening and closing --------------------------------------------

    /// Opens a menu, input, panel or layout. Returns false when an empty
    /// list menu without `<MENU>.EMPTY` was not opened.
    fn open(&mut self, host: &mut dyn Host, target: &str, params: BTreeMap<String, String>) -> Result<bool, String> {
        if self.model.menus.contains_key(target) {
            self.params.insert(target.to_string(), params);
            self.menus.insert(target.to_string(), MenuState::default());
            self.pending.remove(target);
            self.evaluate(host, target, true);
            let empty = self.menus.get(target).is_some_and(|m| m.total() == 0);
            let is_list = matches!(self.model.menus.get(target).map(|m| &m.kind), Some(MenuKind::List(_)));
            if is_list && empty && !self.model.texts.has(&format!("{target}.EMPTY")) {
                return Ok(false);
            }
            self.screens.push(Screen::Menu(target.to_string()));
        } else if let Some(def) = self.model.inputs.get(target) {
            let current =
                if def.kind == InputKind::Password { String::new() } else { self.vars.get(&def.var).map(Value::to_text).unwrap_or_default() };
            let buf: Vec<char> = current.chars().collect();
            self.inputs.insert(target.to_string(), InputState { cursor: buf.len(), buf, error: None, tabbed: false });
            self.screens.push(Screen::Input(target.to_string()));
        } else if self.model.panels.contains_key(target) {
            self.params.insert(target.to_string(), params);
            self.screens.push(Screen::Panel(target.to_string()));
            self.pending.remove(target);
            self.evaluate(host, target, true);
        } else if self.model.layouts.contains_key(target) {
            self.layout = Some(target.to_string());
            self.focus = self.focusables().first().copied().unwrap_or(0);
            self.dirty = true;
        } else {
            return Err(format!("cannot open unknown `{target}`"));
        }
        Ok(true)
    }

    fn is_final(&self, s: &Screen) -> bool {
        matches!(s, Screen::Menu(n) if self.model.menus.get(n).is_some_and(|m| m.final_))
    }

    /// Closes the top screen; a `final` menu stays unless `force`.
    fn close_top(&mut self, force: bool) -> bool {
        match self.screens.last() {
            None => false,
            Some(s) if !force && self.is_final(s) => false,
            Some(_) => {
                match self.screens.pop() {
                    Some(Screen::Menu(name)) if self.mode == Mode::Serve => {
                        self.effects.push(Effect::Event(Event::new("closed", &name, None)));
                    }
                    Some(Screen::Panel(name)) => self.reset_form(&name),
                    _ => {}
                }
                true
            }
        }
    }

    fn is_root(&self) -> bool {
        self.layout.is_none() && self.screens.len() == 1
    }

    /// `back`: closes the top screen; on the root of a widget this ends the
    /// program with status 1.
    fn back(&mut self) {
        if self.screens.last().is_some_and(|s| self.is_final(s)) {
            return;
        }
        if self.is_root() {
            match self.mode {
                Mode::Widget => {
                    self.screens.pop();
                    self.effects.push(Effect::Exit { code: 1, output: None });
                }
                Mode::Serve => {
                    self.close_top(true);
                    self.effects.push(Effect::Quit);
                }
                Mode::App => {}
            }
            return;
        }
        self.close_top(false);
    }

    fn quit(&mut self) {
        match self.mode {
            Mode::Widget => self.effects.push(Effect::Exit { code: 2, output: None }),
            _ => self.effects.push(Effect::Quit),
        }
    }

    /// `close`: closes every open menu (not `final` ones, not a widget root).
    fn close_all(&mut self) {
        if self.is_root() {
            self.back();
            return;
        }
        while !self.screens.is_empty() && !self.is_root() && self.close_top(false) {}
    }

    // ----- input handling -------------------------------------------------

    /// A log showing a `--More--` page takes the next key (D 11.4).
    fn page_logs(&mut self, binding: &Binding) -> bool {
        let Some(name) = self.logs.iter().find(|(_, l)| l.paging).map(|(n, _)| n.clone()) else { return false };
        let stack = self.context_stack();
        let is_back = self.model.keymap.lookup(&stack, binding) == Some("back")
            || matches!(binding, Binding::Key(KeyCode::Esc, _));
        let log = self.logs.get_mut(&name).unwrap();
        if is_back {
            log.unread_from = None;
        } else {
            log.page += 1;
        }
        log.paging = false;
        true
    }

    fn handle_key(&mut self, host: &mut dyn Host, code: KeyCode, mods: Mods) {
        let binding = Binding::key(code, mods);
        if self.page_logs(&binding) {
            return;
        }
        let printable = matches!(code, KeyCode::Char(_)) && !mods.ctrl && !mods.alt;
        if let Some(form) = self.active_form()
            && self.form_key(host, &form, code, mods) {
                return;
            }
        match self.active() {
            Some(Screen::Input(name)) => {
                let is_path = self.model.inputs.get(&name).is_some_and(|d| d.kind == InputKind::Path);
                if is_path && code == KeyCode::Tab && mods == Mods::default() {
                    self.complete(host, &name);
                    return;
                }
                if let Some(st) = self.inputs.get_mut(&name) {
                    st.tabbed = false;
                }
                if self.edit_input(&name, code, mods) {
                    return;
                }
            }
            Some(Screen::Menu(name)) => {
                if let Some(MenuKind::List(l)) = self.model.menus.get(&name).map(|m| &m.kind)
                    && l.filter {
                        let state = self.menus.entry(name.clone()).or_default();
                        match code {
                            KeyCode::Char(c) if printable => {
                                state.push_filter(c);
                                return;
                            }
                            KeyCode::Backspace if !state.filter.is_empty() => {
                                state.pop_filter();
                                return;
                            }
                            _ => {}
                        }
                    }
            }
            _ => {}
        }
        let stack = self.context_stack();
        if let Some(action) = self.model.keymap.lookup(&stack, &binding).map(str::to_string) {
            self.perform_named(host, &action);
            return;
        }
        // Row letters in list menus (D 11.2) when the key is not bound.
        if let (KeyCode::Char(c), true, Some(Screen::Menu(name))) = (code, printable, self.active())
            && matches!(self.model.menus.get(&name).map(|m| &m.kind), Some(MenuKind::List(_))) && c.is_ascii_lowercase() {
                let idx = (c as u8 - b'a') as usize;
                let state = self.menus.entry(name.clone()).or_default();
                let scroll = state.scroll;
                if row_letter(idx).is_some() && state.select_index(scroll + idx) {
                    self.confirm(host);
                }
            }
    }

    // ----- forms (spec §8) -----------------------------------------------

    /// The form being edited: the top modal panel, or the focused layout
    /// panel, when it has input fields.
    fn active_form(&self) -> Option<String> {
        match self.active() {
            Some(Screen::Panel(name)) if self.model.panels.get(&name).is_some_and(|p| p.is_form()) => Some(name),
            _ => None,
        }
    }

    fn form_inputs(&self, form: &str) -> Vec<String> {
        self.model.panels.get(form).map(|p| p.form_inputs()).unwrap_or_default()
    }

    /// Editing keys go to the current field; `tab`, `shift+tab`, `up` and
    /// `down` move between fields (`tab` completes in a `path` field).
    /// Returns false for keys the bindings handle (`confirm`, `back`, …).
    fn form_key(&mut self, host: &mut dyn Host, form: &str, code: KeyCode, mods: Mods) -> bool {
        let inputs = self.form_inputs(form);
        let idx = self.form_field.get(form).copied().unwrap_or(0).min(inputs.len().saturating_sub(1));
        let Some(input) = inputs.get(idx).cloned() else { return false };
        let kind = self.model.inputs.get(&input).map(|i| i.kind);
        let plain = mods == Mods::default();
        let step = match code {
            KeyCode::Tab if plain && kind == Some(InputKind::Path) => {
                self.complete(host, &input);
                return true;
            }
            KeyCode::Tab if plain => 1,
            KeyCode::BackTab => -1,
            KeyCode::Down if plain && kind != Some(InputKind::Choice) => 1,
            KeyCode::Up if plain && kind != Some(InputKind::Choice) => -1,
            _ => {
                self.input_state(&input).tabbed = false;
                let used = self.edit_input(&input, code, mods);
                if used {
                    self.form_error.remove(form);
                }
                return used;
            }
        };
        let next = idx as isize + step;
        let modal = !self.screens.is_empty();
        let n = inputs.len() as isize;
        if !modal && (next < 0 || next >= n) {
            // past the ends of a layout form: on to the next area
            return false;
        }
        self.form_field.insert(form.to_string(), next.rem_euclid(n) as usize);
        true
    }

    /// One confirmation for the whole form: every field is checked first,
    /// then all variables are set and the panel's `then` runs.
    fn submit_form(&mut self, host: &mut dyn Host, form: &str) {
        let inputs = self.form_inputs(form);
        let mut values = Vec::new();
        for (i, name) in inputs.iter().enumerate() {
            let Some(def) = self.model.inputs.get(name).cloned() else { continue };
            let text: String = self.input_state(name).buf.iter().collect();
            let ty = self.model.var(&def.var).map(|v| v.ty.clone()).unwrap_or(VarType::Text);
            let checked = ty.parse_value(&text).and_then(|v| match &def.validate {
                Some(expr) => {
                    let vars = self.public_vars();
                    let scope = Scope::new(&vars).with_value(text.clone());
                    if host.eval_cond(expr, &scope) {
                        Ok(v)
                    } else {
                        Err(self.text(def.error.as_deref().unwrap_or("TERMPLATE.INVALID"), &BTreeMap::new()))
                    }
                }
                None => Ok(v),
            });
            match checked {
                Ok(v) => values.push((def, text, v)),
                Err(msg) => {
                    self.form_field.insert(form.to_string(), i);
                    self.form_error.insert(form.to_string(), msg);
                    return;
                }
            }
        }
        for (def, _, v) in &values {
            self.set_var(&def.var, v.clone());
        }
        self.form_error.remove(form);
        let is_root = self.is_root();
        match self.mode {
            Mode::Widget if is_root => {
                let out = values.iter().map(|(_, t, _)| t.as_str()).collect::<Vec<_>>().join("\n");
                self.effects.push(Effect::Exit { code: 0, output: Some(out) });
                return;
            }
            Mode::Serve => {
                for (def, text, _) in &values {
                    let shown = (def.kind != InputKind::Password).then_some(text.as_str());
                    self.effects.push(Effect::Event(Event::new("input", &def.name, shown)));
                }
                self.effects.push(Effect::Event(Event::new("submit", form, None)));
            }
            _ => {}
        }
        if matches!(self.screens.last(), Some(Screen::Panel(n)) if n == form) && !is_root {
            self.close_top(true);
        }
        self.form_field.remove(form);
        let then = self.model.panels.get(form).and_then(|p| p.then.clone());
        if let Some(then) = then
            && (self.mode != Mode::Serve || !matches!(then, Action::Command(_) | Action::Run { .. })) {
                self.perform(host, &then, None);
            }
    }

    /// Forgets a form's unsaved edits: the fields show their variables.
    fn reset_form(&mut self, form: &str) {
        for i in self.form_inputs(form) {
            self.inputs.remove(&i);
        }
        self.form_field.remove(form);
        self.form_error.remove(form);
    }

    /// `tab` in a `path` input: completes the common prefix of the matches;
    /// a second `tab` lists them (spec §8).
    fn complete(&mut self, host: &mut dyn Host, name: &str) {
        let st = self.input_state(name);
        let typed: String = st.buf.iter().collect();
        let tabbed = st.tabbed;
        let mut matches = host.complete_path(&typed);
        let fill = match matches.len() {
            0 => None,
            1 => matches.pop(),
            _ => Some(common_prefix(&matches)).filter(|c| c.chars().count() > typed.chars().count()),
        };
        if let Some(text) = fill {
            self.set_input_text(name, &text);
        } else if tabbed && matches.len() > 1 {
            if matches.len() > MAX_COMPLETIONS {
                matches.truncate(MAX_COMPLETIONS);
                matches.push(MORE_ROW.to_string());
            }
            let mut params = BTreeMap::new();
            params.insert("matches".to_string(), matches.join("\n"));
            if let Ok(true) = self.open(host, COMPLETE_MENU, params) {
                self.completing = Some(name.to_string());
            }
        }
        if let Some(st) = self.inputs.get_mut(name) {
            st.tabbed = true;
        }
    }

    fn set_input_text(&mut self, name: &str, text: &str) {
        let st = self.input_state(name);
        st.buf = text.chars().collect();
        st.cursor = st.buf.len();
        st.error = None;
    }

    /// Line editing in an input. Returns true when the key was consumed.
    fn edit_input(&mut self, name: &str, code: KeyCode, mods: Mods) -> bool {
        let Some(def) = self.model.inputs.get(name).cloned() else { return false };
        if mods.ctrl || mods.alt {
            return false;
        }
        if def.kind == InputKind::Choice {
            let Some(VarType::Choice(opts)) = self.model.var(&def.var).map(|v| v.ty.clone()) else { return false };
            let st = self.input_state(name);
            let cur: String = st.buf.iter().collect();
            let i = opts.iter().position(|o| *o == cur).unwrap_or(0);
            let next = match code {
                KeyCode::Left | KeyCode::Up => (i + opts.len() - 1) % opts.len(),
                KeyCode::Right | KeyCode::Down | KeyCode::Char(' ') => (i + 1) % opts.len(),
                _ => return false,
            };
            st.buf = opts[next].chars().collect();
            st.cursor = st.buf.len();
            return true;
        }
        let st = self.input_state(name);
        match code {
            KeyCode::Char(c) => {
                if def.kind == InputKind::Number && !(c.is_ascii_digit() || c == '.' || c == '-') {
                    return true;
                }
                st.buf.insert(st.cursor, c);
                st.cursor += 1;
            }
            KeyCode::Backspace if st.cursor > 0 => {
                st.cursor -= 1;
                st.buf.remove(st.cursor);
            }
            KeyCode::Backspace => {}
            KeyCode::Delete if st.cursor < st.buf.len() => {
                st.buf.remove(st.cursor);
            }
            KeyCode::Delete => {}
            KeyCode::Left => st.cursor = st.cursor.saturating_sub(1),
            KeyCode::Right => st.cursor = (st.cursor + 1).min(st.buf.len()),
            KeyCode::Home => st.cursor = 0,
            KeyCode::End => st.cursor = st.buf.len(),
            _ => return false,
        }
        st.error = None;
        true
    }

    fn hit(&self, x: usize, y: usize) -> Option<Hit> {
        self.hits.iter().rev().find(|(r, _)| r.contains(x, y)).map(|(_, h)| h.clone())
    }

    fn modal_open(&self) -> bool {
        !self.screens.is_empty()
    }

    /// The pointer highlights the menu row under it (M 8.1).
    fn hover(&mut self, x: usize, y: usize) {
        if let Some(Hit::MenuRow { menu, index }) = self.hit(x, y)
            && (!self.modal_open() || matches!(self.screens.last(), Some(Screen::Menu(m)) if *m == menu))
                && let Some(m) = self.menus.get_mut(&menu) {
                    m.select_index(index);
                }
    }

    /// Mouse in menus and panels works without bindings (M 8.1): a click on
    /// a row chooses it, a click outside a modal menu is `back`, a click on
    /// a modal panel is `confirm`, the wheel scrolls (or adjusts). Other
    /// mouse input goes through the key bindings.
    fn handle_mouse(&mut self, host: &mut dyn Host, binding: Binding, x: usize, y: usize) {
        if self.model.settings.mouse == Some(false) {
            return;
        }
        if self.page_logs(&binding) {
            return;
        }
        let hit = self.hit(x, y);
        let top_menu = match self.screens.last() {
            Some(Screen::Menu(m)) => Some(m.clone()),
            _ => None,
        };
        let plain = |m: Mouse| binding == Binding::mouse(m);
        match &hit {
            Some(Hit::MenuRow { menu, .. }) if self.modal_open() && top_menu.as_ref() != Some(menu) => return,
            Some(Hit::MenuRow { menu, index }) if plain(Mouse::Click) => {
                self.focus_target(menu);
                if self.menus.get_mut(menu).is_some_and(|m| m.select_index(*index)) {
                    self.confirm(host);
                }
                return;
            }
            Some(Hit::MenuRow { menu, .. }) if plain(Mouse::WheelUp) || plain(Mouse::WheelDown) => {
                self.focus_target(menu);
                let dir = if plain(Mouse::WheelUp) { "left" } else { "right" };
                if self.current_adjust(menu).is_some() {
                    self.nav(dir);
                } else {
                    self.nav(if plain(Mouse::WheelUp) { "up" } else { "down" });
                }
                return;
            }
            Some(Hit::FormField { panel, index }) if plain(Mouse::Click) => {
                // a click picks the field (and the area of a layout form)
                if self.screens.is_empty() {
                    self.focus_target(panel);
                }
                if self.active_form().as_deref() == Some(panel.as_str()) {
                    self.form_field.insert(panel.clone(), *index);
                }
                return;
            }
            Some(Hit::Modal) if plain(Mouse::Click) && self.active_form().is_some() => return,
            Some(Hit::Modal) if plain(Mouse::Click) && matches!(self.screens.last(), Some(Screen::Panel(_))) => {
                self.confirm(host);
                return;
            }
            Some(Hit::Modal) => {}
            _ if plain(Mouse::Click) && top_menu.is_some() => {
                // a click outside the menu is `back` (M 8.1)
                self.nav_back();
                return;
            }
            Some(Hit::Area(i)) if !self.modal_open() => {
                self.focus = *i;
                if plain(Mouse::WheelUp) || plain(Mouse::WheelDown) {
                    self.nav(if plain(Mouse::WheelUp) { "up" } else { "down" });
                    return;
                }
            }
            Some(Hit::Input(name)) if plain(Mouse::Click) && !self.modal_open() => {
                let _ = self.open(host, name, BTreeMap::new());
                return;
            }
            _ => {}
        }
        let stack = self.context_stack();
        if let Some(action) = self.model.keymap.lookup(&stack, &binding).map(str::to_string) {
            self.perform_named(host, &action);
        }
    }

    /// Focusable areas of the current layout in legend order: area N is
    /// the N:th (spec §4).
    fn numbered_areas(&self) -> Vec<String> {
        let Some(base) = &self.layout else { return vec![] };
        let (w, h) = self.size;
        let Some(def) = self.model.choose_layout(base, w, h) else { return vec![] };
        def.order
            .iter()
            .filter_map(|c| def.areas.get(c))
            .map(|a| a.target.clone())
            .filter(|t| self.is_focusable(t))
            .take(9)
            .collect()
    }

    /// `focus N`; does nothing while a modal is open.
    fn focus_area(&mut self, n: usize) {
        if let Some(t) = self.numbered_areas().get(n.wrapping_sub(1)).cloned() {
            self.focus_target(&t);
        }
    }

    fn focus_target(&mut self, target: &str) {
        if self.screens.is_empty()
            && let Some(i) = self.layout_areas().iter().position(|(_, t)| t == target) {
                self.focus = i;
            }
    }

    // ----- actions --------------------------------------------------------

    /// Performs a key-bound action: a built-in, `open X`, `toggle X` or a
    /// command.
    pub fn perform_named(&mut self, host: &mut dyn Host, name: &str) {
        if let Some(target) = name.strip_prefix("open ") {
            // The same binding closes the menu it opened (M 8 ALWAYS).
            if matches!(self.screens.last(), Some(Screen::Menu(m)) if m == target) {
                self.close_top(false);
            } else if let Err(e) = self.open(host, target, BTreeMap::new()) {
                self.report(&e);
            }
            return;
        }
        if let Some(n) = name.strip_prefix("focus ").and_then(|n| n.parse().ok()) {
            self.focus_area(n);
            return;
        }
        if let Some(target) = name.strip_prefix("toggle ") {
            let a = if self.model.var(target).is_some() { Action::Toggle(target.into()) } else { Action::TogglePanel(target.into()) };
            self.perform(host, &a, None);
            return;
        }
        if KEY_ACTIONS.contains(&name) {
            match name {
                "quit" => self.quit(),
                "back" => self.back_or_cancel(host),
                "close" => self.close_all(),
                "confirm" => self.confirm(host),
                "refresh" => self.refresh_all(host),
                "next_area" => self.cycle_focus(1),
                "prev_area" => self.cycle_focus(-1),
                nav => self.nav(nav),
            }
            return;
        }
        if self.mode == Mode::Serve {
            self.effects.push(Effect::Event(Event::new("key", name, None)));
            return;
        }
        let it = self.current_it();
        self.run_command(host, name, it);
    }

    /// The selected row(s) of the active list menu: `IT` for a command
    /// bound to a key (spec §7.1).
    fn current_it(&self) -> Option<String> {
        let Some(Screen::Menu(name)) = self.active() else { return None };
        if !matches!(self.model.menus.get(&name).map(|m| &m.kind), Some(MenuKind::List(_))) {
            return None;
        }
        let sel = self.menus.get(&name)?.selection();
        (!sel.is_empty()).then(|| sel.join("\n"))
    }

    fn cycle_focus(&mut self, dir: isize) {
        if !self.screens.is_empty() {
            return;
        }
        let f = self.focusables();
        if f.is_empty() {
            return;
        }
        let pos = f.iter().position(|&i| i == self.focus).unwrap_or(0) as isize;
        let n = f.len() as isize;
        self.focus = f[((pos + dir) % n + n) as usize % f.len()];
    }

    fn nav_back(&mut self) {
        match self.active() {
            Some(Screen::Menu(name)) => {
                // `back` first clears a filter.
                if let Some(st) = self.menus.get_mut(&name)
                    && !st.filter.is_empty() {
                        st.set_filter("");
                        return;
                    }
                if !self.screens.is_empty() {
                    self.back();
                }
            }
            Some(Screen::Output { title, .. }) if self.screens.is_empty() => {
                if let Some(l) = self.logs.get_mut(&title) {
                    l.back = 0;
                }
            }
            Some(Screen::Panel(name)) if self.screens.is_empty() => self.reset_form(&name),
            _ if !self.screens.is_empty() => self.back(),
            _ => {}
        }
    }

    fn nav(&mut self, nav: &str) {
        match self.active() {
            Some(Screen::Menu(name)) => {
                let page = self.area_rects.get(&name).map(|r| r.h.max(2) as isize - 1).unwrap_or(10);
                if let ("left" | "right", Some((var, lo, hi, pool))) = (nav, self.current_adjust(&name)) {
                    self.adjust(&var, lo, hi, pool.as_deref(), if nav == "left" { -1.0 } else { 1.0 }, false);
                    return;
                }
                let st = self.menus.entry(name.clone()).or_default();
                match nav {
                    "up" => st.move_by(-1, true),
                    "down" => st.move_by(1, true),
                    "page_up" => st.move_by(-page, false),
                    "page_down" => st.move_by(page, false),
                    "home" => st.home(),
                    "end" => st.end(),
                    "select" => {
                        // marks the row and moves on, as in MC and fzf
                        st.toggle_selected();
                        st.move_by(1, false);
                    }
                    _ => {}
                }
            }
            Some(Screen::Output { title, .. }) => {
                let height = self.area_rects.get(&title).map(|r| r.h).unwrap_or(10).max(2);
                if let Some(Screen::Output { scroll, lines, .. }) = self.screens.last_mut() {
                    let max = lines.len().saturating_sub(1);
                    *scroll = match nav {
                        "up" => scroll.saturating_sub(1),
                        "down" => (*scroll + 1).min(max),
                        "page_up" => scroll.saturating_sub(height),
                        "page_down" => (*scroll + height).min(max),
                        "home" => 0,
                        "end" => max,
                        _ => *scroll,
                    };
                } else if let Some(log) = self.logs.get_mut(&title) {
                    let max = log.entries.len().saturating_sub(1);
                    log.back = match nav {
                        "up" => (log.back + 1).min(max),
                        "down" => log.back.saturating_sub(1),
                        "page_up" => (log.back + height).min(max),
                        "page_down" => log.back.saturating_sub(height),
                        "home" => max,
                        "end" => 0,
                        _ => log.back,
                    };
                }
            }
            _ => {}
        }
    }

    fn current_adjust(&self, menu: &str) -> Option<(String, f64, f64, Option<String>)> {
        let key = self.menus.get(menu)?.current()?.key.clone();
        let MenuKind::Static(items) = &self.model.menus.get(menu)?.kind else { return None };
        match &items.iter().find(|i| i.key == key)?.action {
            Action::Adjust { var, lo, hi, pool } => Some((var.clone(), *lo, *hi, pool.clone())),
            _ => None,
        }
    }

    /// Changes an adjustable value by `step` within `lo..hi`. Increases are
    /// paid from `pool` and decreases return to it (M 7.5); `wrap` turns
    /// past the top back to `lo` (confirm on an adjust item).
    fn adjust(&mut self, var: &str, lo: f64, hi: f64, pool: Option<&str>, step: f64, wrap: bool) {
        let cur = self.vars.get(var).and_then(Value::as_number).unwrap_or(lo);
        let mut next = (cur + step).clamp(lo, hi);
        if wrap && cur + step > hi {
            next = lo;
        }
        if let Some(p) = pool {
            let have = self.vars.get(p).and_then(Value::as_number).unwrap_or(0.0);
            let cost = next - cur;
            if cost > have {
                return;
            }
            self.set_var(p, Value::Number(have - cost));
        }
        self.set_var(var, Value::Number(next));
    }

    fn confirm(&mut self, host: &mut dyn Host) {
        match self.active() {
            Some(Screen::Menu(name)) => self.confirm_menu(host, &name),
            Some(Screen::Input(name)) => self.submit_input(host, &name),
            Some(Screen::Confirm { command, it }) => {
                self.screens.pop();
                self.execute_command(host, &command, it);
            }
            Some(Screen::Output { .. }) if !self.screens.is_empty() => {
                self.screens.pop();
            }
            Some(Screen::Output { .. }) => {}
            Some(Screen::Panel(name)) if self.model.panels.get(&name).is_some_and(|p| p.is_form()) => {
                self.submit_form(host, &name);
            }
            Some(Screen::Panel(_)) if !self.screens.is_empty() => self.back(),
            Some(Screen::Panel(_)) => {}
            None => {}
        }
    }

    fn confirm_menu(&mut self, host: &mut dyn Host, name: &str) {
        let Some(def) = self.model.menus.get(name).cloned() else { return };
        let Some(state) = self.menus.get(name) else { return };
        if name == COMPLETE_MENU {
            let chosen = state.current().map(|r| r.key.clone()).filter(|k| k != MORE_ROW);
            if let Some(path) = chosen {
                self.close_top(true);
                if let Some(input) = self.completing.take() {
                    self.set_input_text(&input, &path);
                }
            }
            return;
        }
        let is_root = self.is_root();
        match &def.kind {
            MenuKind::Static(items) => {
                let Some(row) = state.current().filter(|r| !r.disabled) else { return };
                let Some(item) = items.iter().find(|i| i.key == row.key).cloned() else { return };
                let runs_external = matches!(item.action, Action::Command(_) | Action::Run { .. });
                match self.mode {
                    Mode::Widget if runs_external => {
                        self.effects.push(Effect::Exit { code: 0, output: Some(item.key.clone()) });
                        return;
                    }
                    Mode::Serve => {
                        self.effects.push(Effect::Event(Event::new("select", name, Some(&item.key))));
                        if runs_external {
                            return;
                        }
                    }
                    _ => {}
                }
                match &item.action {
                    Action::Adjust { var, lo, hi, pool } => self.adjust(var, *lo, *hi, pool.as_deref(), 1.0, true),
                    a => {
                        let a = a.clone();
                        self.perform(host, &a, None);
                    }
                }
            }
            MenuKind::List(l) => {
                let sel = state.selection();
                if sel.is_empty() {
                    return;
                }
                let value = sel.join("\n");
                if self.mode == Mode::Widget && is_root {
                    self.effects.push(Effect::Exit { code: 0, output: Some(value) });
                    return;
                }
                if self.mode == Mode::Serve {
                    self.effects.push(Effect::Event(Event::new("select", name, Some(&value))));
                }
                if let Some(then) = &l.then
                    && (self.mode != Mode::Serve || !matches!(then, Action::Command(_) | Action::Run { .. })) {
                        self.perform(host, then, Some(value));
                    }
                let top = matches!(self.screens.last(), Some(Screen::Menu(n)) if n == name);
                if l.keep {
                    // a kept list is recomputed after each choice (M 7.5);
                    // the search found its row and ends (spec §7.1)
                    if let Some(st) = self.menus.get_mut(name) {
                        st.set_filter("");
                    }
                    self.eval_menu(host, name);
                } else if top && !is_root {
                    self.close_top(false);
                }
            }
        }
    }

    fn submit_input(&mut self, host: &mut dyn Host, name: &str) {
        let Some(def) = self.model.inputs.get(name).cloned() else { return };
        let value: String = self.input_state(name).buf.iter().collect();
        let ty = self.model.var(&def.var).map(|v| v.ty.clone()).unwrap_or(VarType::Text);
        let parsed = match ty.parse_value(&value) {
            Ok(v) => v,
            Err(e) => {
                self.inputs.entry(name.to_string()).or_default().error = Some(e);
                return;
            }
        };
        if let Some(expr) = &def.validate {
            let vars = self.public_vars();
            let scope = Scope::new(&vars).with_value(value.clone());
            if !host.eval_cond(expr, &scope) {
                let msg = self.text(def.error.as_deref().unwrap_or("TERMPLATE.INVALID"), &BTreeMap::new());
                self.inputs.entry(name.to_string()).or_default().error = Some(msg);
                return;
            }
        }
        self.set_var(&def.var, parsed);
        let is_root = self.is_root();
        let in_modal = matches!(self.screens.last(), Some(Screen::Input(n)) if n == name);
        match self.mode {
            Mode::Widget if is_root => {
                self.effects.push(Effect::Exit { code: 0, output: Some(value) });
                return;
            }
            Mode::Serve => {
                let shown = (def.kind != InputKind::Password).then_some(value.as_str());
                self.effects.push(Effect::Event(Event::new("input", name, shown)));
            }
            _ => {}
        }
        if in_modal && !is_root {
            self.screens.pop();
        }
        if let Some(then) = &def.then
            && (self.mode != Mode::Serve || !matches!(then, Action::Command(_) | Action::Run { .. })) {
                self.perform(host, then, None);
            }
        if def.clear {
            // after `then`, which already took the value (spec §8)
            self.set_var(&def.var, ty.parse_value("").unwrap_or(Value::Text(String::new())));
        }
    }

    fn perform(&mut self, host: &mut dyn Host, action: &Action, it: Option<String>) {
        match action {
            Action::Quit => self.quit(),
            Action::Back => self.back_or_cancel(host),
            Action::Close => self.close_all(),
            Action::Confirm => self.confirm(host),
            Action::Refresh => self.refresh_all(host),
            Action::NextArea => self.cycle_focus(1),
            Action::PrevArea => self.cycle_focus(-1),
            Action::Focus(n) => self.focus_area(*n),
            Action::Nav(n) => self.nav(n),
            Action::Open { target, args } => {
                let params = self.bind_args(host, target, args, it);
                // `open` leaves a final menu (D 11.2).
                match self.open(host, target, params) {
                    Ok(_) => {}
                    Err(e) => self.report(&e),
                }
            }
            Action::Set { var, value } => {
                let vars = self.public_vars();
                let mut scope = Scope::new(&vars);
                scope.it = it;
                let v = host.eval_value(value, &scope).to_text();
                if let Err(e) = self.set_var_text(var, &v) {
                    self.report(&e);
                }
            }
            Action::Toggle(var) => {
                let cur = self.vars.get(var).is_some_and(Value::truthy);
                self.set_var(var, Value::Bool(!cur));
            }
            Action::TogglePanel(p) => {
                if !self.hidden_panels.remove(p) {
                    self.hidden_panels.insert(p.clone());
                }
                self.dirty = true;
            }
            Action::Adjust { var, lo, hi, pool } => self.adjust(var, *lo, *hi, pool.as_deref(), 1.0, true),
            Action::Emit(name) => self.effects.push(Effect::Event(Event::new("emit", name, None))),
            Action::Stop(name) => {
                for j in self.jobs.iter_mut().filter(|j| j.name == *name && !j.cancelled) {
                    j.cancelled = true;
                    host.cancel(j.id);
                }
            }
            Action::Command(name) => self.run_command(host, name, it),
            Action::Run { cmdline, interactive } => {
                if !*interactive && self.already_running(cmdline) {
                    return;
                }
                let req = RunRequest {
                    cmdline: cmdline.clone(),
                    env: self.command_env(&[], it.as_deref()),
                    stdin: None,
                    interactive: *interactive,
                    timeout: if *interactive { None } else { self.model.settings.timeout },
                    workdir: self.model.app.workdir.clone(),
                    ..Default::default()
                };
                if !*interactive && let Some(id) = host.spawn(&req) {
                    let origin = self.origin();
                    self.jobs.push(Job {
                        id,
                        name: cmdline.clone(),
                        def: None,
                        it,
                        origin,
                        stdout: String::new(),
                        stderr: String::new(),
                        depth: self.chain_depth,
                        cancelled: false,
                    });
                    return;
                }
                let out = host.run(&req);
                if !interactive {
                    self.show_output(None, cmdline, &out.stdout, &out.stderr, out.timed_out);
                }
            }
        }
    }

    /// Shows an engine problem to the user: in the first log, or a modal.
    fn report(&mut self, msg: &str) {
        if let Some(area) = self.model.logs.keys().next().cloned() {
            self.log(&area, msg, true);
        } else {
            self.screens.push(Screen::Output { title: "TERMPLATE.OUTPUT".into(), lines: vec![(msg.into(), true)], scroll: 0 });
        }
    }

    pub fn run_command(&mut self, host: &mut dyn Host, name: &str, it: Option<String>) {
        let Some(def) = self.model.commands.get(name) else {
            self.report(&format!("unknown command `{name}`"));
            return;
        };
        if def.confirm.is_some() {
            self.screens.push(Screen::Confirm { command: name.to_string(), it });
            return;
        }
        self.execute_command(host, name, it);
    }

    /// A command's environment (spec §6): only its `env` entries, declared
    /// secrets and `IT`. Variables reach it only through `env`.
    fn command_env(&self, extra: &[(String, String)], it: Option<&str>) -> Vec<(String, String)> {
        let mut env: Vec<(String, String)> = it.map(|it| ("IT".to_string(), it.to_string())).into_iter().collect();
        for (k, v) in extra {
            env.retain(|(ek, _)| ek != k);
            env.push((k.clone(), v.clone()));
        }
        env
    }

    fn execute_command(&mut self, host: &mut dyn Host, name: &str, it: Option<String>) {
        let Some(def) = self.model.commands.get(name).cloned() else { return };
        if self.chain_depth >= MAX_CHAIN {
            self.report(&format!("command chain too deep at `{name}`"));
            return;
        }
        let background = def.run.is_some() && !def.interactive && def.window.is_none();
        if background && self.already_running(name) {
            return;
        }
        self.chain_depth += 1;
        match &def.run {
            Some(cmdline) => {
                let vars = self.public_vars();
                let mut scope = Scope::new(&vars);
                scope.it = it.clone();
                let mut extra: Vec<(String, String)> =
                    def.env.iter().map(|(k, e)| (k.clone(), host.eval_value(e, &scope).to_text())).collect();
                let mut stdin = None;
                for (var, mode) in &def.secrets {
                    let v = self.vars.get(var).map(Value::to_text).unwrap_or_default();
                    match mode {
                        SecretMode::Stdin => stdin = Some(v),
                        SecretMode::Env => extra.push((var.clone(), v)),
                    }
                }
                let req = RunRequest {
                    cmdline: cmdline.clone(),
                    shell: def.shell.clone(),
                    script: def.script,
                    window: def.window,
                    env: self.command_env(&extra, it.as_deref()),
                    stdin,
                    interactive: def.interactive,
                    timeout: if background { self.model.settings.timeout } else { None },
                    workdir: self.model.app.workdir.clone(),
                };
                if def.window.is_some() {
                    // Started and forgotten (spec §15.1): `then` and
                    // `on_error` only tell whether the window opened.
                    let out = host.run(&req);
                    if !out.stderr.is_empty() {
                        self.report(out.stderr.trim_end());
                    }
                    self.continue_chain(host, &def, out.ok(), it);
                    self.chain_depth -= 1;
                    return;
                }
                let spawned = if background { host.spawn(&req) } else { None };
                match spawned {
                    Some(id) => {
                        let origin = self.origin();
                        self.jobs.push(Job {
                            id,
                            name: name.to_string(),
                            def: Some(def.clone()),
                            it,
                            origin,
                            stdout: String::new(),
                            stderr: String::new(),
                            depth: self.chain_depth,
                            cancelled: false,
                        });
                    }
                    None => {
                        let out = host.run(&req);
                        self.finish_command(host, name, &def, it, out);
                    }
                }
            }
            None => self.continue_chain(host, &def, true, it),
        }
        self.chain_depth -= 1;
    }

    /// Handles a finished command: `into`, its output, the failure message,
    /// then `then` or `on_error`.
    fn finish_command(&mut self, host: &mut dyn Host, name: &str, def: &CommandDef, it: Option<String>, out: Outcome) {
        let ok = out.ok();
        if let Some(var) = &def.into {
            let v = out.stdout.strip_suffix('\n').unwrap_or(&out.stdout).to_string();
            if let Err(e) = self.set_var_text(var, &v) {
                self.report(&e);
            }
            if !out.stderr.is_empty() || out.timed_out {
                self.show_output(def.output.as_deref(), name, "", &out.stderr, out.timed_out);
            }
        } else if !def.interactive {
            self.show_output(def.output.as_deref(), name, &out.stdout, &out.stderr, out.timed_out);
        }
        if !ok && def.on_error.is_none() && !out.timed_out {
            let mut extra = BTreeMap::new();
            extra.insert("STATUS".to_string(), out.status.to_string());
            let msg = self.text("TERMPLATE.FAILED", &extra);
            match &def.output {
                Some(area) => self.log(area, &msg, true),
                None => self.append_output(&msg, true),
            }
        }
        self.continue_chain(host, def, ok, it);
    }

    fn continue_chain(&mut self, host: &mut dyn Host, def: &CommandDef, ok: bool, it: Option<String>) {
        let next = if ok { def.then.clone() } else { def.on_error.clone() };
        if let Some(a) = next {
            self.perform(host, &a, it);
        }
    }

    fn show_output(&mut self, area: Option<&str>, title: &str, stdout: &str, stderr: &str, timed_out: bool) {
        let timeout_msg = timed_out.then(|| self.text("TERMPLATE.TIMEOUT", &BTreeMap::new()));
        match area {
            Some(area) => {
                let area = area.to_string();
                if !stdout.is_empty() {
                    self.log(&area, stdout, false);
                }
                if !stderr.is_empty() {
                    self.log(&area, stderr, true);
                }
                if let Some(m) = timeout_msg {
                    self.log(&area, &m, true);
                }
            }
            None => {
                let mut lines: Vec<(String, bool)> = stdout.lines().map(|l| (strip_ansi(l), false)).collect();
                lines.extend(stderr.lines().map(|l| (strip_ansi(l), true)));
                if let Some(m) = timeout_msg {
                    lines.push((m, true));
                }
                if !lines.is_empty() {
                    self.screens.push(Screen::Output { title: title.to_string(), lines, scroll: 0 });
                }
            }
        }
    }

    fn append_output(&mut self, line: &str, stderr: bool) {
        if let Some(Screen::Output { lines, .. }) = self.screens.last_mut() {
            lines.push((line.to_string(), stderr));
        } else {
            self.screens.push(Screen::Output { title: "TERMPLATE.OUTPUT".into(), lines: vec![(line.to_string(), stderr)], scroll: 0 });
        }
    }

    // ----- drawing --------------------------------------------------------

    pub fn render(&mut self) -> Frame {
        let (w, h) = self.size;
        let mut f = Frame::new(w, h);
        self.hits.clear();
        if let Some(base) = self.layout.clone()
            && let Some(def) = self.model.choose_layout(&base, w, h).cloned() {
                let solved = def.grid.solve(&self.model.area_sizes(&def, w, h), w, h);
                if solved.too_small {
                    let mut extra = BTreeMap::new();
                    extra.insert("WIDTH".to_string(), solved.min_width.to_string());
                    extra.insert("HEIGHT".to_string(), solved.min_height.to_string());
                    let msg = self.text("TERMPLATE.TOO_SMALL", &extra);
                    f.text(0, 0, w, &msg, Role::Warning);
                    return f;
                }
                let areas = self.layout_areas();
                let numbered = if self.model.settings.area_numbers { self.numbered_areas() } else { vec![] };
                for (i, (rect, target)) in areas.iter().enumerate() {
                    self.hits.push((*rect, Hit::Area(i)));
                    let focused = self.screens.is_empty() && i == self.focus;
                    let mut inner = *rect;
                    f.set_area(self.chain_for(target));
                    if let Some(border) = self.model.area_border(target) {
                        f.border(*rect, border, Role::Border);
                        inner = Rect::new(rect.x + 1, rect.y + 1, rect.w.saturating_sub(2), rect.h.saturating_sub(2));
                        if let Some(n) = numbered.iter().position(|t| t == target) {
                            f.text(rect.x + 1, rect.y, rect.w.saturating_sub(2), &format!("{}", n + 1), Role::Title);
                        }
                    }
                    self.area_rects.insert(target.clone(), inner);
                    self.draw_target(&mut f, target, inner, focused);
                    if self.jobs.iter().any(|j| j.origin.as_deref() == Some(target.as_str())) && rect.w > 0 {
                        // top-right corner, on the border when there is one
                        let x = rect.x + rect.w - if inner.w < rect.w { 2 } else { 1 };
                        f.text(x, rect.y, 1, &self.spinner().to_string(), Role::Status);
                    }
                    f.set_area(vec![]);
                }
            }
        let screens = self.screens.clone();
        for (i, s) in screens.iter().enumerate() {
            let top = i + 1 == screens.len();
            self.draw_modal(&mut f, s, top, i);
            f.set_area(vec![]);
        }
        let areas: BTreeSet<String> = self.layout_areas().into_iter().map(|(_, t)| t).collect();
        if w > 0 && self.jobs.iter().any(|j| j.origin.as_ref().is_none_or(|o| !areas.contains(o))) {
            f.text(w - 1, 0, 1, &self.spinner().to_string(), Role::Status);
        }
        f
    }

    fn spinner(&self) -> char {
        SPINNER[self.spin % SPINNER.len()]
    }

    /// The style chain of a target: its key context (for menus) and name.
    fn chain_for(&self, target: &str) -> Vec<String> {
        match self.model.menus.get(target) {
            Some(m) => vec![m.context().to_string(), target.to_string()],
            None if self.model.inputs.contains_key(target) => vec!["INPUT".into(), target.to_string()],
            None => vec![target.to_string()],
        }
    }

    fn draw_target(&mut self, f: &mut Frame, target: &str, rect: Rect, focused: bool) {
        if let Some(def) = self.model.panels.get(target) {
            let height = def.height;
            if !self.hidden_panels.contains(target) {
                self.draw_panel(f, target, rect);
                if let Some(e) = self.form_error.get(target).cloned() {
                    // below the drawing when there is room, else on its last row
                    let y = rect.y + height.min(rect.h.saturating_sub(1));
                    f.aligned(rect.x, y, rect.w, &e, Align::Left, Role::Warning);
                }
            }
        } else if self.model.menus.contains_key(target) {
            self.draw_menu(f, target, rect, focused, true);
        } else if self.model.logs.contains_key(target) {
            self.draw_log(f, target, rect);
        } else if self.model.inputs.contains_key(target) {
            self.draw_input_field(f, target, rect, focused);
        }
    }

    fn draw_panel(&mut self, f: &mut Frame, name: &str, rect: Rect) {
        let Some(def) = self.model.panels.get(name).cloned() else { return };
        for (y, row) in def.picture.iter().enumerate().take(rect.h) {
            for (x, ch) in row.iter().enumerate().take(rect.w) {
                f.put(rect.x + x, rect.y + y, *ch, Role::Border);
            }
        }
        let values = self.panel_values.get(name).cloned().unwrap_or_default();
        let editing = self.active_form().as_deref() == Some(name);
        let current = self.form_field.get(name).copied().unwrap_or(0);
        let mut input_no = 0;
        for field in &def.fields {
            let v = values.get(&field.name).cloned().unwrap_or_default();
            for r in &field.rects {
                if r.x >= rect.w || r.y >= rect.h {
                    continue;
                }
                let (x, y) = (rect.x + r.x, rect.y + r.y);
                let w = r.w.min(rect.w - r.x);
                let hgt = r.h.min(rect.h - r.y);
                match &field.kind {
                    FieldKind::Bar { .. } => {
                        let (v, m) = v.split_once(' ').unwrap_or(("0", "0"));
                        let (v, m): (f64, f64) = (v.parse().unwrap_or(0.0), m.parse().unwrap_or(0.0));
                        let frac = if m > 0.0 { (v / m).clamp(0.0, 1.0) } else { 0.0 };
                        let filled = (frac * w as f64).round() as usize;
                        for i in 0..w {
                            f.put(x + i, y, if i < filled { '█' } else { '░' }, Role::Bar);
                        }
                    }
                    FieldKind::Spinner { .. } => {
                        let c = if v == "true" { SPINNER[self.spin % SPINNER.len()] } else { ' ' };
                        f.put(x, y, c, Role::Field);
                    }
                    FieldKind::Input { input } => {
                        // a form field shows its edit buffer (spec §8)
                        let masked = self.model.inputs.get(input).is_some_and(|i| i.kind == InputKind::Password);
                        let st = self.input_state(input).clone();
                        let text: String = if masked { "*".repeat(st.buf.len()) } else { st.buf.iter().collect() };
                        let here = editing && input_no == current;
                        let role = if here { Role::Selected } else { Role::Input };
                        let (start, cursor_x) = input_view(&text, st.cursor, w);
                        let visible: String = text.chars().skip(start).collect();
                        f.fill(Rect::new(x, y, w, 1), ' ', role);
                        f.text(x, y, w, &visible, role);
                        if here {
                            f.cursor = Some((x + cursor_x, y));
                        }
                        self.hits.push((Rect::new(x, y, w, 1), Hit::FormField { panel: name.to_string(), index: input_no }));
                    }
                    _ if hgt > 1 => {
                        // multi-line fields wrap between words; the last
                        // visible line is cut with `…` (D 11.5)
                        let mut lines = wrap(&v, w);
                        if lines.len() > hgt {
                            lines.truncate(hgt);
                            let last = lines.last_mut().unwrap();
                            last.push_str(" …");
                        }
                        for (i, line) in lines.iter().chain(std::iter::repeat(&String::new())).take(hgt).enumerate() {
                            f.aligned(x, y + i, w, line, field.align, Role::Field);
                        }
                    }
                    _ => f.aligned(x, y, w, &v, field.align, Role::Field),
                }
            }
            if matches!(field.kind, FieldKind::Input { .. }) {
                input_no += 1;
            }
        }
    }

    fn menu_lines(&self, name: &str) -> (Option<String>, Option<String>, Option<String>, bool) {
        let title = self.text_opt(&format!("{name}.TITLE"));
        let text = self.text_opt(&format!("{name}.TEXT"));
        let (header, filter) = match self.model.menus.get(name).map(|d| &d.kind) {
            Some(MenuKind::List(l)) => {
                let header = (!l.columns.is_empty()).then(|| {
                    l.columns.iter().map(|c| self.text(&format!("{name}.{}", c.name), &BTreeMap::new())).collect::<Vec<_>>().join(" ")
                });
                (header, l.filter)
            }
            _ => (None, false),
        };
        (title, text, header, filter)
    }

    fn draw_menu(&mut self, f: &mut Frame, name: &str, rect: Rect, focused: bool, show_title: bool) {
        let Some(def) = self.model.menus.get(name).cloned() else { return };
        let (title, text, _, filter) = self.menu_lines(name);
        let title = title.filter(|_| show_title);
        let mut y = rect.y;
        let bottom = rect.y + rect.h;
        if let Some(t) = &title {
            f.aligned(rect.x, y, rect.w, t, Align::Left, Role::Title);
            y += 1;
        }
        if let Some(t) = &text
            && y < bottom {
                f.aligned(rect.x, y, rect.w, t, Align::Left, Role::Normal);
                y += 1;
            }
        let list = match &def.kind {
            MenuKind::List(l) => Some(l.clone()),
            _ => None,
        };
        let prefix_w = self.row_prefix_width(name);
        let widths = list.as_ref().map(|l| column_widths(&l.columns, rect.w.saturating_sub(prefix_w)));
        if let (Some(l), Some(widths)) = (&list, &widths)
            && !l.columns.is_empty() && y < bottom {
                let mut x = rect.x + prefix_w;
                for (c, w) in l.columns.iter().zip(widths) {
                    let label = self.text(&format!("{name}.{}", c.name), &BTreeMap::new());
                    f.aligned(x, y, *w, &label, c.align, Role::Header);
                    x += w + 1;
                }
                y += 1;
            }
        let rows_h = bottom.saturating_sub(y).saturating_sub(usize::from(filter));
        let state = self.menus.entry(name.to_string()).or_default();
        state.scroll_into_view(rows_h);
        let state = state.clone();
        if state.is_empty() && rows_h > 0 {
            let key = format!("{name}.EMPTY");
            let msg = if !self.loaded.contains(name) {
                // the first evaluation is still running
                self.text("TERMPLATE.LOADING", &BTreeMap::new())
            } else if self.model.texts.has(&key) {
                self.text(&key, &BTreeMap::new())
            } else {
                self.text("TERMPLATE.EMPTY", &BTreeMap::new())
            };
            f.aligned(rect.x, y, rect.w, &msg, Align::Left, Role::Disabled);
        }
        let multi = list.as_ref().is_some_and(|l| l.multi);
        let letters = list.as_ref().is_some_and(|l| !l.filter);
        for (i, row) in state.rows().enumerate().skip(state.scroll).take(rows_h) {
            let ry = y + i - state.scroll;
            let selected = i == state.cursor;
            let role = if row.disabled {
                Role::Disabled
            } else if selected && focused {
                Role::Selected
            } else if selected {
                Role::Dim
            } else {
                Role::Normal
            };
            let mut prefix = String::new();
            if letters {
                prefix.push(row_letter(i - state.scroll).unwrap_or(' '));
                prefix.push(' ');
            }
            if multi {
                prefix.push_str(if state.selected.contains(&row.key) { "[x] " } else { "[ ] " });
            }
            f.fill(Rect::new(rect.x, ry, rect.w, 1), ' ', role);
            let mut x = rect.x + f.text(rect.x, ry, rect.w, &prefix, role);
            match (&list, &widths) {
                (Some(l), Some(widths)) if !l.columns.is_empty() => {
                    for ((c, w), v) in l.columns.iter().zip(widths).zip(&row.columns) {
                        if x >= rect.x + rect.w {
                            break;
                        }
                        let w = (*w).min(rect.x + rect.w - x);
                        f.aligned(x, ry, w, v, c.align, role);
                        x += w + 1;
                    }
                }
                _ => {
                    let w = rect.x + rect.w - x;
                    f.aligned(x, ry, w, &row.label, Align::Left, role);
                }
            }
            self.hits.push((Rect::new(rect.x, ry, rect.w, 1), Hit::MenuRow { menu: name.to_string(), index: i }));
        }
        if filter && rect.h > 0 {
            f.aligned(rect.x, bottom - 1, rect.w, &format!("/{}", state.filter), Align::Left, Role::Input);
        }
    }

    fn row_prefix_width(&self, name: &str) -> usize {
        match self.model.menus.get(name).map(|m| &m.kind) {
            Some(MenuKind::List(l)) => (if l.filter { 0 } else { 2 }) + if l.multi { 4 } else { 0 },
            _ => 0,
        }
    }

    fn log_text(e: &LogEntry) -> String {
        if e.count > 1 { format!("{} (×{})", e.text, e.count) } else { e.text.clone() }
    }

    fn draw_log(&mut self, f: &mut Frame, name: &str, rect: Rect) {
        let more_key = format!("{name}.MORE");
        let more_text =
            if self.model.texts.has(&more_key) { self.text(&more_key, &BTreeMap::new()) } else { self.text("TERMPLATE.MORE", &BTreeMap::new()) };
        let Some(log) = self.logs.get_mut(name) else { return };
        log.paging = false;
        let role = |stderr: bool| if stderr { Role::Stderr } else { Role::Normal };
        // `more` paging: new messages that do not fit are shown in pages,
        // the oldest first, with `<LOG>.MORE` on the last row (D 11.4).
        if let Some(from) = log.unread_from {
            let lines: Vec<(String, bool)> = log
                .entries
                .iter()
                .skip(from)
                .flat_map(|e| wrap(&Self::log_text(e), rect.w).into_iter().map(move |l| (l, e.stderr)))
                .collect();
            let per = rect.h.saturating_sub(1).max(1);
            if lines.len() > rect.h && (log.page + 1) * per < lines.len() {
                for (i, (l, err)) in lines.iter().skip(log.page * per).take(per).enumerate() {
                    f.aligned(rect.x, rect.y + i, rect.w, l, Align::Left, role(*err));
                }
                f.aligned(rect.x, rect.y + rect.h - 1, rect.w, &more_text, Align::Left, Role::Status);
                log.paging = true;
                return;
            }
            if lines.len() > rect.h {
                // last page: show it, then the log is back to normal
                let start = log.page * per;
                for (i, (l, err)) in lines.iter().skip(start).take(rect.h).enumerate() {
                    f.aligned(rect.x, rect.y + i, rect.w, l, Align::Left, role(*err));
                }
                log.unread_from = None;
                return;
            }
            log.unread_from = None;
        }
        // Newest at the bottom; a message too tall for the area shows its
        // first lines and is cut with `…` (D 11.4).
        let end = log.entries.len().saturating_sub(log.back);
        let mut rows: Vec<(String, bool)> = Vec::new();
        for e in log.entries.iter().take(end).rev() {
            if rows.len() >= rect.h {
                break;
            }
            let mut lines = wrap(&Self::log_text(e), rect.w);
            let room = rect.h - rows.len();
            if lines.len() > room {
                lines.truncate(room);
                let last = lines.last_mut().unwrap();
                *last = crate::frame::fit(&format!("{last} …"), rect.w, Align::Left).trim_end().to_string();
                if !last.ends_with('…') {
                    last.push('…');
                }
            }
            for l in lines.into_iter().rev() {
                rows.push((l, e.stderr));
            }
        }
        rows.reverse();
        for (i, (l, err)) in rows.iter().enumerate() {
            f.aligned(rect.x, rect.y + i, rect.w, l, Align::Left, role(*err));
        }
    }

    /// An input in a layout area: edited in place when focused (spec §8).
    fn draw_input_field(&mut self, f: &mut Frame, name: &str, rect: Rect, focused: bool) {
        let Some(def) = self.model.inputs.get(name).cloned() else { return };
        let st = self.input_state(name).clone();
        let text: String = if def.kind == InputKind::Password { "*".repeat(st.buf.len()) } else { st.buf.iter().collect() };
        let role = if focused { Role::Selected } else { Role::Input };
        let (start, cursor_x) = input_view(&text, st.cursor, rect.w);
        let visible: String = text.chars().skip(start).collect();
        f.fill(Rect::new(rect.x, rect.y, rect.w, 1), ' ', role);
        f.text(rect.x, rect.y, rect.w, &visible, role);
        if focused && self.screens.is_empty() {
            f.cursor = Some((rect.x + cursor_x, rect.y));
        }
        if let Some(e) = st.error.as_ref().filter(|_| rect.h > 1) {
            f.aligned(rect.x, rect.y + 1, rect.w, e, Align::Left, Role::Warning);
        }
        self.hits.push((Rect::new(rect.x, rect.y, rect.w, 1), Hit::Input(name.to_string())));
    }

    fn modal_border(&self, chain: &[String]) -> Border {
        self.model.style.border(chain).unwrap_or(Border::Single)
    }

    fn draw_modal(&mut self, f: &mut Frame, s: &Screen, top: bool, depth: usize) {
        let (w, h) = self.size;
        let max_w = w.saturating_sub(4).max(10);
        let max_h = h.saturating_sub(2).max(3);
        match s {
            Screen::Menu(name) => {
                let chain = self.chain_for(name);
                f.set_area(chain.clone());
                let state = self.menus.get(name).cloned().unwrap_or_default();
                let (title, text, header, filter) = self.menu_lines(name);
                let prefix = self.row_prefix_width(name);
                let empty = state.is_empty().then(|| {
                    let key = format!("{name}.EMPTY");
                    self.text(if self.model.texts.has(&key) { &key } else { "TERMPLATE.EMPTY" }, &BTreeMap::new())
                });
                let content_w = state
                    .rows()
                    .map(|r| str_width(&r.label) + prefix)
                    .chain(empty.iter().map(|t| str_width(t)))
                    .chain(title.iter().map(|t| str_width(t) + 2))
                    .chain(text.iter().map(|t| str_width(t)))
                    .chain(header.iter().map(|t| str_width(t) + prefix))
                    .max()
                    .unwrap_or(10)
                    .max(12);
                let extra = usize::from(text.is_some()) + usize::from(header.is_some()) + usize::from(filter);
                let rows = state.len().max(1);
                let def_size = self.model.menus.get(name).map(|m| m.size).unwrap_or_default();
                let bw = def_size.0.map(|d| d.resolve(w)).unwrap_or(content_w + 4).min(max_w);
                let bh = def_size.1.map(|d| d.resolve(h)).unwrap_or(rows + extra + 2).min(max_h);
                let r = centered(w, h, bw, bh, depth);
                let border = self.modal_border(&chain);
                self.draw_box(f, r, border, title.as_deref());
                let inner = Rect::new(r.x + 2, r.y + 1, r.w.saturating_sub(4), r.h.saturating_sub(2));
                self.area_rects.insert(name.clone(), inner);
                self.draw_menu(f, name, inner, top, false);
            }
            Screen::Input(name) => {
                let chain = self.chain_for(name);
                f.set_area(chain.clone());
                let Some(def) = self.model.inputs.get(name).cloned() else { return };
                let st = self.inputs.get(name).cloned().unwrap_or_default();
                let title = self.text_opt(&format!("{name}.TITLE"));
                let prompt = self.text_opt(&format!("{name}.PROMPT"));
                let bw = 44.min(max_w);
                let bh = (3 + usize::from(prompt.is_some()) + usize::from(st.error.is_some())).min(max_h);
                let r = centered(w, h, bw, bh, depth);
                let border = self.modal_border(&chain);
                self.draw_box(f, r, border, title.as_deref());
                let mut y = r.y + 1;
                let iw = r.w.saturating_sub(4);
                if let Some(p) = &prompt {
                    f.aligned(r.x + 2, y, iw, p, Align::Left, Role::Normal);
                    y += 1;
                }
                let text: String = if def.kind == InputKind::Password { "*".repeat(st.buf.len()) } else { st.buf.iter().collect() };
                let (start, cursor_x) = input_view(&text, st.cursor, iw);
                let visible: String = text.chars().skip(start).collect();
                // inputs scroll instead of cutting with `…`
                f.fill(Rect::new(r.x + 2, y, iw, 1), ' ', Role::Input);
                f.text(r.x + 2, y, iw, &visible, Role::Input);
                if top {
                    f.cursor = Some((r.x + 2 + cursor_x, y));
                }
                if let Some(e) = &st.error {
                    f.aligned(r.x + 2, y + 1, iw, e, Align::Left, Role::Warning);
                }
            }
            Screen::Panel(name) => {
                // a modal box always has a border from the style (spec §11);
                // a form has a line for its hint or error
                let ctx = self.screen_context(s);
                let chain = vec![ctx, name.clone()];
                f.set_area(chain.clone());
                let Some(def) = self.model.panels.get(name) else { return };
                let form = def.is_form();
                let (dw, dh) = (def.width, def.height);
                let message = form.then(|| match self.form_error.get(name) {
                    Some(e) => (e.clone(), Role::Warning),
                    None => (self.text("TERMPLATE.FORM_HINT", &BTreeMap::new()), Role::Dim),
                });
                let mw = message.as_ref().map_or(0, |(m, _)| str_width(m));
                let (pw, ph) = (dw.max(mw) + 4, dh + 2 + usize::from(form));
                let r = centered(w, h, pw.min(w), ph.min(h), depth);
                let title = self.text_opt(&format!("{name}.TITLE"));
                let border = self.modal_border(&chain);
                self.draw_box(f, r, border, title.as_deref());
                let name = name.clone();
                let inner = Rect::new(r.x + 2, r.y + 1, r.w.saturating_sub(4), r.h.saturating_sub(2 + usize::from(form)));
                self.draw_panel(f, &name, inner);
                if let Some((msg, role)) = message {
                    f.aligned(r.x + 2, r.y + r.h - 2, r.w.saturating_sub(4), &msg, Align::Left, role);
                }
            }
            Screen::Confirm { command, it } => {
                f.set_area(vec!["MENU".into()]);
                let key = self.model.commands.get(command).and_then(|c| c.confirm.clone()).unwrap_or_default();
                let extra: BTreeMap<String, String> = it.iter().map(|v| ("IT".to_string(), v.clone())).collect();
                let msg = self.text(&key, &extra);
                let hint = self.text("TERMPLATE.CONFIRM_HINT", &BTreeMap::new());
                let lines: Vec<&str> = msg.lines().collect();
                let cw = lines.iter().map(|l| str_width(l)).chain([str_width(&hint)]).max().unwrap_or(10);
                let r = centered(w, h, (cw + 4).min(max_w), (lines.len() + 3).min(max_h), depth);
                let border = self.modal_border(&["MENU".to_string()]);
                self.draw_box(f, r, border, None);
                for (i, l) in lines.iter().enumerate() {
                    f.aligned(r.x + 2, r.y + 1 + i, r.w.saturating_sub(4), l, Align::Left, Role::Normal);
                }
                f.aligned(r.x + 2, r.y + r.h - 2, r.w.saturating_sub(4), &hint, Align::Left, Role::Dim);
            }
            Screen::Output { title, lines, scroll } => {
                f.set_area(vec!["MENU".into()]);
                let title = self.text(title, &BTreeMap::new());
                let cw = lines.iter().map(|(l, _)| str_width(l)).chain([str_width(&title) + 2]).max().unwrap_or(10);
                let r = centered(w, h, (cw + 4).min(max_w), (lines.len() + 2).min(max_h), depth);
                let border = self.modal_border(&["MENU".to_string()]);
                self.draw_box(f, r, border, Some(&title));
                let body = r.h.saturating_sub(2);
                for (i, (l, err)) in lines.iter().skip(*scroll).take(body).enumerate() {
                    f.aligned(r.x + 2, r.y + 1 + i, r.w.saturating_sub(4), l, Align::Left, if *err { Role::Stderr } else { Role::Normal });
                }
            }
        }
    }

    fn draw_box(&mut self, f: &mut Frame, r: Rect, border: Border, title: Option<&str>) {
        f.fill(r, ' ', Role::Normal);
        f.border(r, if border == Border::None { Border::Single } else { border }, Role::Border);
        if let Some(t) = title {
            f.text(r.x + 2, r.y, r.w.saturating_sub(4), &format!(" {t} "), Role::Title);
        }
        self.hits.push((r, Hit::Modal));
    }
}

fn centered(w: usize, h: usize, bw: usize, bh: usize, depth: usize) -> Rect {
    let bw = bw.min(w);
    let bh = bh.min(h);
    let x = (w - bw) / 2 + depth.min(4);
    let y = (h - bh) / 2 + depth.min(4) / 2;
    Rect::new(x.min(w - bw), y.min(h - bh), bw, bh)
}

/// Column widths: fixed widths as given, `*` columns share the rest.
pub fn column_widths(cols: &[crate::model::Column], total: usize) -> Vec<usize> {
    let gaps = cols.len().saturating_sub(1);
    let fixed: usize = cols.iter().filter_map(|c| c.width).sum();
    let stars = cols.iter().filter(|c| c.width.is_none()).count();
    let rest = total.saturating_sub(fixed + gaps);
    let mut given = 0;
    let mut star_i = 0;
    cols.iter()
        .map(|c| match c.width {
            Some(w) => w,
            None => {
                star_i += 1;
                let share = if star_i == stars { rest - given } else { rest / stars.max(1) };
                given += share;
                share
            }
        })
        .collect()
}

/// Numeric comparison when both sides are numbers, text otherwise.
fn compare_values(a: &str, b: &str) -> std::cmp::Ordering {
    match (a.trim().parse::<f64>(), b.trim().parse::<f64>()) {
        (Ok(x), Ok(y)) => x.partial_cmp(&y).unwrap_or(std::cmp::Ordering::Equal),
        _ => a.cmp(b),
    }
}

/// Removes ANSI escape sequences (CSI and OSC).
pub fn strip_ansi(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '\x1b' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('[') => {
                for c in chars.by_ref() {
                    if ('@'..='~').contains(&c) {
                        break;
                    }
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
    out
}

#[cfg(test)]
mod tests;
