//! The shell host: evaluates §3 expressions and runs commands.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::Arc;
use std::time::Duration;

use termplate_core::{eval_now, EvalRequest, Expr, ExprKind, Host, JobEvent, JobId, OwnedScope, Outcome, RunRequest, Scope, Value};

use crate::exec::CommandRunner;
use crate::expr::{compare, parse_cond, parse_template, Cond, Part, Template};

/// Timeout for `$( … )` substitutions when no setting is given.
pub const DEFAULT_SUBST_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Debug, Clone)]
enum Parsed {
    Template(Template),
    Cond(Cond),
}

pub struct ShellHost<R: CommandRunner> {
    pub runner: R,
    pub subst_timeout: Duration,
    pub workdir: Option<String>,
    cache: HashMap<(String, bool), Parsed>,
    jobs: HashMap<JobId, Arc<AtomicBool>>,
    next_job: JobId,
    events: (Sender<JobEvent>, Receiver<JobEvent>),
}

impl<R: CommandRunner> ShellHost<R> {
    pub fn new(runner: R) -> Self {
        ShellHost {
            runner,
            subst_timeout: DEFAULT_SUBST_TIMEOUT,
            workdir: None,
            cache: HashMap::new(),
            jobs: HashMap::new(),
            next_job: 1,
            events: channel(),
        }
    }

    fn template(&mut self, src: &str) -> Template {
        let key = (src.to_string(), false);
        if let Some(Parsed::Template(t)) = self.cache.get(&key) {
            return t.clone();
        }
        // Loading already checked the expression; fall back to a literal.
        let t = parse_template(src).unwrap_or_else(|_| Template(vec![Part::Lit(src.to_string())]));
        self.cache.insert(key, Parsed::Template(t.clone()));
        t
    }

    fn cond(&mut self, src: &str) -> Option<Cond> {
        let key = (src.to_string(), true);
        if let Some(Parsed::Cond(c)) = self.cache.get(&key) {
            return Some(c.clone());
        }
        let c = parse_cond(src).ok()?;
        self.cache.insert(key, Parsed::Cond(c.clone()));
        Some(c)
    }

    fn substitute(&mut self, cmd: &str, scope: &Scope) -> Outcome {
        let req = RunRequest {
            cmdline: cmd.to_string(),
            env: scope.env(),
            stdin: None,
            interactive: false,
            timeout: Some(self.subst_timeout),
            workdir: self.workdir.clone(),
            ..Default::default()
        };
        self.runner.run(&req)
    }

    /// True when evaluating `expr` runs a command, so it is worth doing in
    /// the background.
    fn runs_commands(&mut self, expr: &Expr, kind: ExprKind) -> bool {
        match kind {
            ExprKind::Cond => self.cond(&expr.src).is_some_and(|c| c.runs_commands()),
            ExprKind::Value | ExprKind::Source => self.template(&expr.src).runs_commands(),
        }
    }

    fn lookup(name: &str, scope: &Scope) -> Value {
        scope
            .get(name)
            .or_else(|| std::env::var(name).ok().map(Value::Text))
            .unwrap_or(Value::Text(String::new()))
    }

    pub fn eval_template(&mut self, t: &Template, scope: &Scope) -> Value {
        if let Some(v) = t.single_var() {
            return Self::lookup(v, scope);
        }
        let mut out = String::new();
        for p in &t.0 {
            match p {
                Part::Lit(s) => out.push_str(s),
                Part::Var(v) => out.push_str(&Self::lookup(v, scope).to_text()),
                Part::Cmd(c) => {
                    let o = self.substitute(c, scope);
                    out.push_str(o.stdout.trim_end_matches('\n'));
                }
            }
        }
        Value::Text(out)
    }

    fn eval(&mut self, c: &Cond, scope: &Scope) -> bool {
        match c {
            Cond::Or(items) => items.iter().any(|i| self.eval(i, scope)),
            Cond::And(items) => items.iter().all(|i| self.eval(i, scope)),
            Cond::Not(i) => !self.eval(i, scope),
            Cond::Ok(cmd) => self.substitute(cmd, scope).ok(),
            Cond::Truthy(t) => self.eval_template(t, scope).truthy(),
            Cond::Cmp(a, op, b) => {
                let a = self.eval_template(a, scope).to_text();
                let b = self.eval_template(b, scope).to_text();
                compare(&a, *op, &b)
            }
        }
    }
}

impl<R: CommandRunner> Host for ShellHost<R> {
    fn check(&mut self, expr: &Expr, kind: ExprKind) -> Result<(), String> {
        match kind {
            ExprKind::Cond => {
                let c = parse_cond(&expr.src)?;
                self.cache.insert((expr.src.clone(), true), Parsed::Cond(c));
            }
            ExprKind::Value | ExprKind::Source => {
                let t = parse_template(&expr.src)?;
                self.cache.insert((expr.src.clone(), false), Parsed::Template(t));
            }
        }
        Ok(())
    }

    fn eval_value(&mut self, expr: &Expr, scope: &Scope) -> Value {
        let t = self.template(&expr.src);
        self.eval_template(&t, scope)
    }

    fn eval_cond(&mut self, expr: &Expr, scope: &Scope) -> bool {
        match self.cond(&expr.src) {
            Some(c) => self.eval(&c, scope),
            None => false,
        }
    }

    fn eval_source(&mut self, expr: &Expr, scope: &Scope) -> Vec<Value> {
        let t = self.template(&expr.src);
        match self.eval_template(&t, scope) {
            Value::List(items) => items.into_iter().map(Value::Text).collect(),
            v => v.to_text().lines().filter(|l| !l.is_empty()).map(|l| Value::Text(l.to_string())).collect(),
        }
    }

    fn run(&mut self, req: &RunRequest) -> Outcome {
        self.runner.run(req)
    }

    fn spawn(&mut self, req: &RunRequest) -> Option<JobId> {
        let id = self.next_job;
        let cancel = Arc::new(AtomicBool::new(false));
        if !self.runner.spawn(req, id, self.events.0.clone(), cancel.clone()) {
            return None;
        }
        self.next_job += 1;
        self.jobs.insert(id, cancel);
        Some(id)
    }

    fn poll_jobs(&mut self) -> Vec<JobEvent> {
        let events: Vec<JobEvent> = self.events.1.try_iter().collect();
        for e in &events {
            if let JobEvent::Done { job, .. } = e {
                self.jobs.remove(job);
            }
        }
        events
    }

    fn cancel(&mut self, job: JobId) {
        if let Some(c) = self.jobs.get(&job) {
            c.store(true, Ordering::SeqCst);
        }
    }

    /// Evaluates on a thread of its own when the request runs commands;
    /// plain `$NAME` lookups stay here (they are instant).
    fn eval_async(&mut self, req: &EvalRequest, scope: &Scope) -> Option<JobId> {
        let exprs: Vec<(Expr, ExprKind)> = req.exprs().map(|(e, k)| (e.clone(), k)).collect();
        if !exprs.iter().any(|(e, k)| self.runs_commands(e, *k)) {
            return None;
        }
        let mut worker = ShellHost::new(self.runner.worker()?);
        worker.subst_timeout = self.subst_timeout;
        worker.workdir = self.workdir.clone();
        worker.cache = self.cache.clone();
        let id = self.next_job;
        self.next_job += 1;
        let (req, scope, events) = (req.clone(), OwnedScope::of(scope), self.events.0.clone());
        std::thread::spawn(move || {
            let results = eval_now(&mut worker, &req, &scope.scope());
            let _ = events.send(JobEvent::Evaluated { job: id, results });
        });
        Some(id)
    }

    fn complete_path(&mut self, prefix: &str) -> Vec<String> {
        crate::fs::complete_path(prefix, self.workdir.as_deref(), std::env::var("HOME").ok().as_deref())
    }

    /// The interpreter must exist; `sh`, `bash` and `zsh` scripts are
    /// checked with `-n`, and the error's line number is the script's.
    fn check_script(&mut self, shell: &str, script: Option<&str>) -> Result<(), (Option<usize>, String)> {
        if crate::fs::find_program(shell, std::env::var("PATH").ok().as_deref()).is_none() {
            return Err((None, format!("interpreter `{shell}` not found")));
        }
        let Some(script) = script else { return Ok(()) };
        if !matches!(shell.rsplit('/').next(), Some("sh" | "bash" | "zsh")) {
            return Ok(());
        }
        let file = crate::exec::temp_file(script).map_err(|e| (None, format!("cannot write the script file: {e}")))?;
        let path = file.path().to_string_lossy().into_owned();
        let out = std::process::Command::new(shell)
            .arg("-n")
            .arg(&path)
            .stdin(std::process::Stdio::null())
            .output()
            .map_err(|e| (None, format!("cannot run `{shell} -n`: {e}")))?;
        if out.status.success() {
            return Ok(());
        }
        Err(syntax_error(&String::from_utf8_lossy(&out.stderr), &path))
    }
}

/// The first line of a `-n` check's error without the file name, and the
/// line number in it: `F: line 5: …` (bash, sh), `F:5: …` (zsh).
fn syntax_error(stderr: &str, path: &str) -> (Option<usize>, String) {
    let first = stderr.lines().find(|l| !l.trim().is_empty()).unwrap_or("syntax error");
    let rest = first.strip_prefix(path).unwrap_or(first).trim_start_matches([':', ' ']);
    let digits: String = rest.chars().skip_while(|c| !c.is_ascii_digit()).take_while(char::is_ascii_digit).collect();
    let line = digits.parse().ok();
    // drop the `line 5:` / `5:` part, keep the message
    let msg = match rest.find(&digits).filter(|_| line.is_some()) {
        Some(i) => rest[i + digits.len()..].trim_start_matches([':', ' ']),
        None => rest,
    };
    (line, msg.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::exec::ProcessRunner;
    use termplate_core::value::Vars;

    /// Records substitutions instead of running them.
    #[derive(Default)]
    struct Fake {
        outputs: HashMap<String, Outcome>,
        seen: Vec<RunRequest>,
    }

    impl CommandRunner for Fake {
        fn run(&mut self, req: &RunRequest) -> Outcome {
            self.seen.push(req.clone());
            self.outputs.get(&req.cmdline).cloned().unwrap_or_default()
        }
    }

    fn vars() -> Vars {
        let mut v = Vars::new();
        v.insert("TARGET".into(), Value::Text("/mnt/backup".into()));
        v.insert("LEVEL".into(), Value::Number(3.0));
        v.insert("VERBOSE".into(), Value::Bool(false));
        v.insert("FILES".into(), Value::List(vec!["a".into(), "b c".into()]));
        v
    }

    fn e(s: &str) -> Expr {
        Expr::bare(s)
    }

    #[test]
    fn syntax_errors_name_the_script_line() {
        let p = "/tmp/termplate-1.sh";
        assert_eq!(syntax_error(&format!("{p}: line 5: syntax error near `fi'\n{p}: line 5: `fi fi'\n"), p), (Some(5), "syntax error near `fi'".into()));
        assert_eq!(syntax_error(&format!("{p}:2: unmatched \"\n"), p), (Some(2), "unmatched \"".into()));
        assert_eq!(syntax_error(&format!("{p}: rivi 1: odottamaton EOF\n"), p), (Some(1), "odottamaton EOF".into()));
        assert_eq!(syntax_error("weird\n", p), (None, "weird".into()));
    }

    #[test]
    fn script_check_runs_the_interpreter() {
        let mut h = ShellHost::new(ProcessRunner::default());
        assert!(h.check_script("sh", Some("echo ok\n")).is_ok());
        let (line, msg) = h.check_script("sh", Some("echo a\n\nif true; then\n  echo b\nfi fi\n")).unwrap_err();
        assert_eq!(line, Some(5), "{msg}");
        assert!(h.check_script("no-such-interpreter-xyz", None).unwrap_err().1.contains("not found"));
        assert!(h.check_script("/bin/sh", None).is_ok());
    }

    #[test]
    fn values_and_templates() {
        let v = vars();
        let s = Scope::new(&v).with_it("x.log");
        let mut h = ShellHost::new(Fake::default());
        assert_eq!(h.eval_value(&e("$TARGET"), &s), Value::Text("/mnt/backup".into()));
        assert_eq!(h.eval_value(&e("$LEVEL"), &s), Value::Number(3.0), "single var keeps type");
        assert_eq!(h.eval_value(&e("FILE: $it"), &s), Value::Text("FILE: x.log".into()));
        assert_eq!(h.eval_value(&e("\"$TARGET/sub\""), &s), Value::Text("/mnt/backup/sub".into()));
        assert_eq!(h.eval_value(&e("$UNDEFINED_TERMPLATE_VAR"), &s), Value::Text("".into()));
    }

    #[test]
    fn substitution_runs_with_scope_env_and_strips_newlines() {
        let v = vars();
        let s = Scope::new(&v);
        let mut fake = Fake::default();
        fake.outputs.insert("df -h".into(), Outcome { stdout: "12G\n\n".into(), ..Default::default() });
        let mut h = ShellHost::new(fake);
        h.subst_timeout = Duration::from_secs(3);
        assert_eq!(h.eval_value(&e("free $(df -h)"), &s), Value::Text("free 12G".into()));
        let req = &h.runner.seen[0];
        assert!(req.env.contains(&("TARGET".into(), "/mnt/backup".into())));
        assert_eq!(req.timeout, Some(Duration::from_secs(3)));
    }

    #[test]
    fn conditions() {
        let v = vars();
        let s = Scope::new(&v);
        let mut h = ShellHost::new(Fake::default());
        assert!(!h.eval_cond(&e("$LEVEL > 5"), &s));
        assert!(h.eval_cond(&e("$LEVEL >= 3"), &s));
        assert!(h.eval_cond(&e("$LEVEL < 10"), &s), "numeric, not text, comparison");
        assert!(!h.eval_cond(&e("$TARGET == \"\""), &s));
        assert!(h.eval_cond(&e("not $VERBOSE"), &s));
        assert!(h.eval_cond(&e("$VERBOSE or $LEVEL == 3"), &s));
        assert!(!h.eval_cond(&e("$VERBOSE and $LEVEL == 3"), &s));
    }

    #[test]
    fn ok_uses_exit_status() {
        let v = vars();
        let s = Scope::new(&v);
        let mut fake = Fake::default();
        fake.outputs.insert("false".into(), Outcome { status: 1, ..Default::default() });
        let mut h = ShellHost::new(fake);
        assert!(h.eval_cond(&e("ok $(true)"), &s));
        assert!(!h.eval_cond(&e("ok $(false)"), &s));
        assert!(h.eval_cond(&e("not ok $(false)"), &s));
    }

    #[test]
    fn sources_from_lists_and_output() {
        let v = vars();
        let s = Scope::new(&v);
        let mut fake = Fake::default();
        fake.outputs.insert("ls".into(), Outcome { stdout: "x\n\ny\n".into(), ..Default::default() });
        let mut h = ShellHost::new(fake);
        let texts = |vals: Vec<Value>| vals.iter().map(Value::to_text).collect::<Vec<_>>();
        assert_eq!(texts(h.eval_source(&e("$FILES"), &s)), ["a", "b c"]);
        assert_eq!(texts(h.eval_source(&e("$(ls)"), &s)), ["x", "y"], "empty lines dropped");
    }

    #[test]
    fn check_reports_syntax_errors() {
        let mut h = ShellHost::new(Fake::default());
        assert!(h.check(&e("$LEVEL >"), ExprKind::Cond).is_err());
        assert!(h.check(&e("$(unterminated"), ExprKind::Value).is_err());
        assert!(h.check(&e("$LEVEL > 5"), ExprKind::Cond).is_ok());
    }

    #[test]
    fn real_shell_substitution_sees_variables() {
        let v = vars();
        let s = Scope::new(&v);
        let mut h = ShellHost::new(ProcessRunner::default());
        assert_eq!(h.eval_value(&e("$(printf '%s' \"$TARGET\")"), &s), Value::Text("/mnt/backup".into()));
        assert!(h.eval_cond(&e("ok $(test \"$LEVEL\" = 3)"), &s));
        assert!(!h.eval_cond(&e("ok $(test -d /definitely/not/here)"), &s));
    }

    fn wait_eval(h: &mut ShellHost<ProcessRunner>) -> termplate_core::EvalResults {
        let t = std::time::Instant::now();
        loop {
            for e in h.poll_jobs() {
                if let JobEvent::Evaluated { results, .. } = e {
                    return results;
                }
            }
            assert!(t.elapsed() < Duration::from_secs(5), "no result");
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    #[test]
    fn commands_are_evaluated_in_the_background() {
        use termplate_core::{EvalResults, Evaluated};
        let v = vars();
        let s = Scope::new(&v);
        let mut h = ShellHost::new(ProcessRunner::default());
        let req = EvalRequest::Batch(vec![
            (e("$(sleep 0.3; printf '%s' \"$TARGET\")"), ExprKind::Value),
            (e("$LEVEL"), ExprKind::Value),
            (e("ok $(true)"), ExprKind::Cond),
        ]);
        let t = std::time::Instant::now();
        assert!(h.eval_async(&req, &s).is_some());
        assert!(t.elapsed() < Duration::from_millis(200), "returns before the command ends");
        let expected = EvalResults::Batch(vec![
            Evaluated::Value(Value::Text("/mnt/backup".into())),
            Evaluated::Value(Value::Number(3.0)),
            Evaluated::Cond(true),
        ]);
        assert_eq!(wait_eval(&mut h), expected);
    }

    #[test]
    fn rows_are_evaluated_per_item() {
        use termplate_core::{EvalResults, Evaluated};
        let v = vars();
        let s = Scope::new(&v);
        let mut h = ShellHost::new(ProcessRunner::default());
        let req = EvalRequest::Rows { source: e("$(printf 'a\\nb\\n')"), each: vec![(e("$(printf '%s!' \"$IT\")"), ExprKind::Value)] };
        h.eval_async(&req, &s).unwrap();
        let rows = |it: &str| (it.to_string(), vec![Evaluated::Value(Value::Text(format!("{it}!")))]);
        assert_eq!(wait_eval(&mut h), EvalResults::Rows(vec![rows("a"), rows("b")]));
    }

    #[test]
    fn plain_lookups_and_unshareable_runners_stay_synchronous() {
        let v = vars();
        let s = Scope::new(&v);
        let plain = EvalRequest::Batch(vec![(e("$TARGET"), ExprKind::Value), (e("$LEVEL > 2"), ExprKind::Cond)]);
        assert!(ShellHost::new(ProcessRunner::default()).eval_async(&plain, &s).is_none());
        let cmd = EvalRequest::Batch(vec![(e("$(date)"), ExprKind::Value)]);
        assert!(ShellHost::new(Fake::default()).eval_async(&cmd, &s).is_none(), "Fake has no worker");
    }
}
