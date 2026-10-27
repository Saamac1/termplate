//! The host interface (spec §16). The core keeps expressions as raw text;
//! the host parses them at load time ([`Host::check`]) and evaluates them.

use std::collections::BTreeMap;
use std::time::Duration;

use crate::value::{Value, Vars};

#[derive(Debug, Clone, PartialEq)]
pub struct Expr {
    pub src: String,
    pub file: String,
    pub line: usize,
}

impl Expr {
    pub fn new(src: impl Into<String>, file: impl Into<String>, line: usize) -> Self {
        Expr { src: src.into(), file: file.into(), line }
    }
    /// An expression without a source location (tests, protocol).
    pub fn bare(src: impl Into<String>) -> Self {
        Expr::new(src, "", 0)
    }
}

/// What an expression is used for; decides how the host validates it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExprKind {
    Value,
    Cond,
    Source,
}

/// Evaluation context: variables, the parameters of the menu or panel
/// (`menu L(dir)` → `$dir`), and the special names of list menus (`$it`)
/// and input validation (`$VALUE`).
#[derive(Debug, Clone, Default)]
pub struct Scope<'a> {
    pub vars: Option<&'a Vars>,
    pub params: Option<&'a BTreeMap<String, String>>,
    pub it: Option<String>,
    pub value: Option<String>,
}

impl<'a> Scope<'a> {
    pub fn new(vars: &'a Vars) -> Self {
        Scope { vars: Some(vars), params: None, it: None, value: None }
    }
    pub fn with_params(mut self, params: &'a BTreeMap<String, String>) -> Self {
        self.params = Some(params);
        self
    }
    pub fn with_it(mut self, it: impl Into<String>) -> Self {
        self.it = Some(it.into());
        self
    }
    pub fn with_value(mut self, v: impl Into<String>) -> Self {
        self.value = Some(v.into());
        self
    }

    /// Looks a name up: `it`, `IT`, `VALUE`, then variables.
    pub fn get(&self, name: &str) -> Option<Value> {
        match name {
            "it" | "IT" if self.it.is_some() => self.it.clone().map(Value::Text),
            "VALUE" if self.value.is_some() => self.value.clone().map(Value::Text),
            _ => self
                .params
                .and_then(|p| p.get(name).cloned().map(Value::Text))
                .or_else(|| self.vars.and_then(|v| v.get(name).cloned())),
        }
    }

    /// Environment for a command: every variable and parameter plus
    /// `IT`/`VALUE`.
    pub fn env(&self) -> Vec<(String, String)> {
        let mut env: Vec<(String, String)> =
            self.vars.map(|v| v.iter().map(|(k, v)| (k.clone(), v.to_text())).collect()).unwrap_or_default();
        if let Some(p) = self.params {
            env.extend(p.iter().map(|(k, v)| (k.clone(), v.clone())));
        }
        if let Some(it) = &self.it {
            env.push(("IT".into(), it.clone()));
        }
        if let Some(v) = &self.value {
            env.push(("VALUE".into(), v.clone()));
        }
        env
    }
}

/// A [`Scope`] that owns its values, for evaluation on another thread.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct OwnedScope {
    pub vars: Vars,
    pub params: BTreeMap<String, String>,
    pub it: Option<String>,
    pub value: Option<String>,
}

impl OwnedScope {
    pub fn of(scope: &Scope) -> Self {
        OwnedScope {
            vars: scope.vars.cloned().unwrap_or_default(),
            params: scope.params.cloned().unwrap_or_default(),
            it: scope.it.clone(),
            value: scope.value.clone(),
        }
    }

    pub fn scope(&self) -> Scope<'_> {
        Scope { vars: Some(&self.vars), params: Some(&self.params), it: self.it.clone(), value: self.value.clone() }
    }
}

/// Expressions a menu or panel needs, evaluated together (spec §5, §7).
#[derive(Debug, Clone, PartialEq)]
pub enum EvalRequest {
    /// Each expression in order.
    Batch(Vec<(Expr, ExprKind)>),
    /// A list menu: the source, then `each` for every item with `$it` set.
    Rows { source: Expr, each: Vec<(Expr, ExprKind)> },
}

impl EvalRequest {
    pub fn exprs(&self) -> Box<dyn Iterator<Item = (&Expr, ExprKind)> + '_> {
        match self {
            EvalRequest::Batch(v) => Box::new(v.iter().map(|(e, k)| (e, *k))),
            EvalRequest::Rows { source, each } => {
                Box::new(std::iter::once((source, ExprKind::Source)).chain(each.iter().map(|(e, k)| (e, *k))))
            }
        }
    }
}

/// One evaluated expression.
#[derive(Debug, Clone, PartialEq)]
pub enum Evaluated {
    Value(Value),
    Cond(bool),
    Source(Vec<Value>),
}

#[derive(Debug, Clone, PartialEq)]
pub enum EvalResults {
    Batch(Vec<Evaluated>),
    /// Each source item with the results of `each`.
    Rows(Vec<(String, Vec<Evaluated>)>),
}

fn eval_one(host: &mut dyn Host, e: &Expr, kind: ExprKind, scope: &Scope) -> Evaluated {
    match kind {
        ExprKind::Value => Evaluated::Value(host.eval_value(e, scope)),
        ExprKind::Cond => Evaluated::Cond(host.eval_cond(e, scope)),
        ExprKind::Source => Evaluated::Source(host.eval_source(e, scope)),
    }
}

/// Evaluates `req` here and now; what [`Host::eval_async`] does elsewhere.
pub fn eval_now(host: &mut dyn Host, req: &EvalRequest, scope: &Scope) -> EvalResults {
    match req {
        EvalRequest::Batch(items) => EvalResults::Batch(items.iter().map(|(e, k)| eval_one(host, e, *k, scope)).collect()),
        EvalRequest::Rows { source, each } => {
            let items: Vec<String> = host.eval_source(source, scope).iter().map(Value::to_text).collect();
            EvalResults::Rows(
                items
                    .into_iter()
                    .map(|it| {
                        let row_scope = Scope { it: Some(it.clone()), ..scope.clone() };
                        let r = each.iter().map(|(e, k)| eval_one(host, e, *k, &row_scope)).collect();
                        (it, r)
                    })
                    .collect(),
            )
        }
    }
}

/// A fully resolved command run.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct RunRequest {
    pub cmdline: String,
    pub env: Vec<(String, String)>,
    pub stdin: Option<String>,
    pub interactive: bool,
    pub timeout: Option<Duration>,
    pub workdir: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Outcome {
    pub status: i32,
    pub stdout: String,
    pub stderr: String,
    pub timed_out: bool,
}

impl Outcome {
    pub fn ok(&self) -> bool {
        self.status == 0 && !self.timed_out
    }
}

/// A command started with [`Host::spawn`].
pub type JobId = u64;

/// Progress of background commands (spec §15).
#[derive(Debug, Clone, PartialEq)]
pub enum JobEvent {
    /// One line of output, without the line end.
    Line { job: JobId, text: String, stderr: bool },
    /// The command ended. The output came earlier as `Line` events, so the
    /// outcome's `stdout` and `stderr` are empty.
    Done { job: JobId, outcome: Outcome },
    /// Results of [`Host::eval_async`].
    Evaluated { job: JobId, results: EvalResults },
}

pub trait Host {
    /// Load-time validation of an expression.
    fn check(&mut self, expr: &Expr, kind: ExprKind) -> Result<(), String>;
    fn eval_value(&mut self, expr: &Expr, scope: &Scope) -> Value;
    fn eval_cond(&mut self, expr: &Expr, scope: &Scope) -> bool;
    fn eval_source(&mut self, expr: &Expr, scope: &Scope) -> Vec<Value>;
    fn run(&mut self, req: &RunRequest) -> Outcome;

    /// Starts a non-interactive command in the background. A host that
    /// cannot do that returns `None`, and the engine uses [`Host::run`].
    fn spawn(&mut self, _req: &RunRequest) -> Option<JobId> {
        None
    }
    /// Output and completions since the last call.
    fn poll_jobs(&mut self) -> Vec<JobEvent> {
        Vec::new()
    }
    /// Asks a background command to stop (SIGTERM, then SIGKILL). It still
    /// ends with a `Done` event.
    fn cancel(&mut self, _job: JobId) {}

    /// Evaluates `req` in the background when that is worth it (it runs
    /// commands); the results come from [`Host::poll_jobs`]. `None` means
    /// the engine evaluates it at once with [`eval_now`].
    fn eval_async(&mut self, _req: &EvalRequest, _scope: &Scope) -> Option<JobId> {
        None
    }

    /// Completions of a typed path for `path` inputs (spec §8): whole
    /// paths starting with `prefix`, directories ending in `/`, sorted.
    fn complete_path(&mut self, _prefix: &str) -> Vec<String> {
        Vec::new()
    }
}

/// A deterministic host for tests. Understands `$NAME`, `"literal"`, bare
/// literals, `not X`, `X == Y`, `true`/`false`; command substitutions are
/// answered from `outputs`, and runs are recorded and answered from
/// `run_results` (default: success with empty output).
#[derive(Debug, Default)]
pub struct TestHost {
    pub outputs: BTreeMap<String, String>,
    pub run_results: BTreeMap<String, Outcome>,
    pub runs: Vec<RunRequest>,
    pub reject: Vec<String>,
    /// With `background`, commands are spawned as jobs that stay running
    /// until [`TestHost::finish`].
    pub background: bool,
    pub jobs: Vec<(JobId, RunRequest)>,
    pub cancelled: Vec<JobId>,
    pub events: Vec<JobEvent>,
    /// Paths offered by `complete_path`.
    pub paths: Vec<String>,
    /// With `async_eval`, `eval_async` queues requests until
    /// [`TestHost::finish_evals`].
    pub async_eval: bool,
    pub evals: Vec<(JobId, EvalRequest, OwnedScope)>,
}

impl TestHost {
    pub fn new() -> Self {
        Self::default()
    }

    /// Ends a background job with its `run_results` entry: its output as
    /// lines, then `Done`.
    pub fn finish(&mut self, job: JobId) {
        let Some(pos) = self.jobs.iter().position(|(id, _)| *id == job) else { return };
        let (_, req) = self.jobs.remove(pos);
        let out = self.run_results.get(&req.cmdline).cloned().unwrap_or_default();
        for (text, stderr) in [(&out.stdout, false), (&out.stderr, true)] {
            for l in text.lines() {
                self.events.push(JobEvent::Line { job, text: l.to_string(), stderr });
            }
        }
        let status = if self.cancelled.contains(&job) { 143 } else { out.status };
        self.events.push(JobEvent::Done { job, outcome: Outcome { status, timed_out: out.timed_out, ..Default::default() } });
    }

    /// Answers the queued `eval_async` requests.
    pub fn finish_evals(&mut self) {
        for (job, req, scope) in std::mem::take(&mut self.evals) {
            let results = eval_now(self, &req, &scope.scope());
            self.events.push(JobEvent::Evaluated { job, results });
        }
    }

    /// The newest running job.
    pub fn last_job(&self) -> Option<JobId> {
        self.jobs.last().map(|(id, _)| *id)
    }

    fn atom(&self, s: &str, scope: &Scope) -> Value {
        let s = s.trim();
        if let Some(name) = s.strip_prefix('$') {
            if name.starts_with('(') {
                return Value::Text(self.outputs.get(s).cloned().unwrap_or_default());
            }
            return scope.get(name).unwrap_or(Value::Text(String::new()));
        }
        if s.len() >= 2 && s.starts_with('"') && s.ends_with('"') {
            return Value::Text(s[1..s.len() - 1].to_string());
        }
        match s {
            "true" => Value::Bool(true),
            "false" => Value::Bool(false),
            _ => s.parse::<f64>().map(Value::Number).unwrap_or_else(|_| Value::Text(s.to_string())),
        }
    }
}

impl Host for TestHost {
    fn check(&mut self, expr: &Expr, _kind: ExprKind) -> Result<(), String> {
        if self.reject.contains(&expr.src) { Err(format!("bad expression `{}`", expr.src)) } else { Ok(()) }
    }

    fn eval_value(&mut self, expr: &Expr, scope: &Scope) -> Value {
        self.atom(&expr.src, scope)
    }

    fn eval_cond(&mut self, expr: &Expr, scope: &Scope) -> bool {
        let s = expr.src.trim();
        if let Some(rest) = s.strip_prefix("not ") {
            return !self.eval_cond(&Expr::bare(rest), scope);
        }
        if let Some(rest) = s.strip_prefix("ok ") {
            return self.outputs.contains_key(rest.trim());
        }
        if let Some((a, b)) = s.split_once("==") {
            return self.atom(a, scope).to_text() == self.atom(b, scope).to_text();
        }
        self.atom(s, scope).truthy()
    }

    fn eval_source(&mut self, expr: &Expr, scope: &Scope) -> Vec<Value> {
        self.atom(&expr.src, scope).as_list().into_iter().map(Value::Text).collect()
    }

    fn run(&mut self, req: &RunRequest) -> Outcome {
        self.runs.push(req.clone());
        self.run_results.get(&req.cmdline).cloned().unwrap_or_default()
    }

    fn spawn(&mut self, req: &RunRequest) -> Option<JobId> {
        if !self.background {
            return None;
        }
        self.runs.push(req.clone());
        let id = self.runs.len() as JobId;
        self.jobs.push((id, req.clone()));
        Some(id)
    }

    fn poll_jobs(&mut self) -> Vec<JobEvent> {
        std::mem::take(&mut self.events)
    }

    fn cancel(&mut self, job: JobId) {
        self.cancelled.push(job);
    }

    fn eval_async(&mut self, req: &EvalRequest, scope: &Scope) -> Option<JobId> {
        if !self.async_eval {
            return None;
        }
        let id = 1000 + self.evals.len() as JobId + self.runs.len() as JobId;
        self.evals.push((id, req.clone(), OwnedScope::of(scope)));
        Some(id)
    }

    fn complete_path(&mut self, prefix: &str) -> Vec<String> {
        self.paths.iter().filter(|p| p.starts_with(prefix)).cloned().collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vars() -> Vars {
        let mut v = Vars::new();
        v.insert("A".into(), Value::Text("x".into()));
        v.insert("N".into(), Value::Number(2.0));
        v
    }

    #[test]
    fn scope_lookup_order() {
        let v = vars();
        let s = Scope::new(&v).with_it("row").with_value("typed");
        assert_eq!(s.get("it"), Some(Value::Text("row".into())));
        assert_eq!(s.get("IT"), Some(Value::Text("row".into())));
        assert_eq!(s.get("VALUE"), Some(Value::Text("typed".into())));
        assert_eq!(s.get("A"), Some(Value::Text("x".into())));
        assert_eq!(s.get("missing"), None);
    }

    #[test]
    fn scope_params_shadow_vars_and_reach_env() {
        let v = vars();
        let mut p = BTreeMap::new();
        p.insert("dir".to_string(), "/tmp".to_string());
        p.insert("A".to_string(), "param".to_string());
        let s = Scope::new(&v).with_params(&p);
        assert_eq!(s.get("dir"), Some(Value::Text("/tmp".into())));
        assert_eq!(s.get("A"), Some(Value::Text("param".into())));
        assert!(s.env().contains(&("dir".into(), "/tmp".into())));
    }

    #[test]
    fn scope_env_contains_vars_and_it() {
        let v = vars();
        let env = Scope::new(&v).with_it("r").env();
        assert!(env.contains(&("N".into(), "2".into())));
        assert!(env.contains(&("IT".into(), "r".into())));
    }

    #[test]
    fn outcome_ok() {
        assert!(Outcome::default().ok());
        assert!(!Outcome { status: 1, ..Default::default() }.ok());
        assert!(!Outcome { timed_out: true, ..Default::default() }.ok());
    }

    #[test]
    fn test_host_conditions() {
        let v = vars();
        let s = Scope::new(&v);
        let mut h = TestHost::new();
        assert!(h.eval_cond(&Expr::bare("$A == x"), &s));
        assert!(h.eval_cond(&Expr::bare("not $MISSING"), &s));
        h.outputs.insert("$(true)".into(), String::new());
        assert!(h.eval_cond(&Expr::bare("ok $(true)"), &s));
        assert!(!h.eval_cond(&Expr::bare("ok $(false)"), &s));
    }
}
