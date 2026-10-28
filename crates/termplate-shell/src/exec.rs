//! Running commands (spec §15): `sh -c` (or the `- shell` interpreter) with
//! values in the environment, `run:` scripts from a temporary file, a
//! timeout that sends SIGTERM and then SIGKILL to the whole process group,
//! background commands that stream their output, terminal hand-over for
//! interactive commands, and `- window` commands (see [`crate::window`]).

use std::io::{BufRead, BufReader, Read, Write};
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::Sender;
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use termplate_core::{JobEvent, JobId, Outcome, RunRequest};

/// Time between SIGTERM and SIGKILL.
pub const KILL_GRACE: Duration = Duration::from_secs(2);

pub trait CommandRunner {
    fn run(&mut self, req: &RunRequest) -> Outcome;

    /// Starts `req` in the background: its output lines and then its end
    /// are sent to `events` as job `id`; setting `cancel` stops it. Returns
    /// false when this runner cannot run in the background.
    fn spawn(&mut self, _req: &RunRequest, _id: JobId, _events: Sender<JobEvent>, _cancel: Arc<AtomicBool>) -> bool {
        false
    }

    /// A runner for another thread (background evaluation); `None` when
    /// this one cannot be shared.
    fn worker(&self) -> Option<Box<dyn CommandRunner + Send>> {
        None
    }
}

impl CommandRunner for Box<dyn CommandRunner + Send> {
    fn run(&mut self, req: &RunRequest) -> Outcome {
        (**self).run(req)
    }
}

/// Called with `true` before an interactive command gets the terminal and
/// with `false` after it returns.
pub type TerminalHandover = Arc<dyn Fn(bool) + Send + Sync>;

#[derive(Clone, Default)]
pub struct ProcessRunner {
    pub handover: Option<TerminalHandover>,
    pub kill_grace: Option<Duration>,
}

impl CommandRunner for ProcessRunner {
    fn run(&mut self, req: &RunRequest) -> Outcome {
        if let Some(kind) = req.window {
            let handover = self.handover.clone();
            return crate::window::open(req, kind, &|r: &RunRequest| {
                if let Some(h) = &handover {
                    h(true);
                }
                let out = run_interactive(r);
                if let Some(h) = &handover {
                    h(false);
                }
                out
            });
        }
        if req.interactive {
            if let Some(h) = &self.handover {
                h(true);
            }
            let out = run_interactive(req);
            if let Some(h) = &self.handover {
                h(false);
            }
            return out;
        }
        run_captured(req, self.kill_grace.unwrap_or(KILL_GRACE))
    }

    fn worker(&self) -> Option<Box<dyn CommandRunner + Send>> {
        Some(Box::new(self.clone()))
    }

    fn spawn(&mut self, req: &RunRequest, id: JobId, events: Sender<JobEvent>, cancel: Arc<AtomicBool>) -> bool {
        let grace = self.kill_grace.unwrap_or(KILL_GRACE);
        let req = req.clone();
        thread::spawn(move || {
            let outcome = run_streaming(&req, grace, id, &events, &cancel);
            let _ = events.send(JobEvent::Done { job: id, outcome });
        });
        true
    }
}

/// A file or folder removed when dropped.
#[derive(Debug)]
pub struct TempPath(PathBuf);

impl TempPath {
    pub fn path(&self) -> &Path {
        &self.0
    }
    /// Leaves the file in place; someone else removes it.
    pub fn keep(self) -> PathBuf {
        let p = self.0.clone();
        std::mem::forget(self);
        p
    }
}

impl Drop for TempPath {
    fn drop(&mut self) {
        if self.0.is_dir() {
            let _ = std::fs::remove_dir_all(&self.0);
        } else {
            let _ = std::fs::remove_file(&self.0);
        }
    }
}

fn temp_name(suffix: &str) -> PathBuf {
    static N: AtomicU64 = AtomicU64::new(0);
    let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.subsec_nanos()).unwrap_or(0);
    std::env::temp_dir().join(format!("termplate-{}-{}-{nanos}{suffix}", std::process::id(), N.fetch_add(1, Ordering::SeqCst)))
}

/// A new file only we can read (mode 600) holding `content`.
pub fn temp_file(content: &str) -> std::io::Result<TempPath> {
    let path = temp_name(".sh");
    let mut f = std::fs::OpenOptions::new().write(true).create_new(true).mode(0o600).open(&path)?;
    let t = TempPath(path);
    f.write_all(content.as_bytes())?;
    Ok(t)
}

/// A new folder only we can enter (mode 700).
pub fn temp_dir() -> std::io::Result<TempPath> {
    let path = temp_name("");
    std::fs::DirBuilder::new().mode(0o700).create(&path)?;
    Ok(TempPath(path))
}

pub fn shell_of(req: &RunRequest) -> &str {
    req.shell.as_deref().unwrap_or("sh")
}

/// The process for `req`: `SHELL -c LINE`, or `SHELL FILE` for a script,
/// whose temporary file lives as long as the returned guard.
fn command(req: &RunRequest) -> Result<(Command, Option<TempPath>), Outcome> {
    let mut cmd = Command::new(shell_of(req));
    let script = if req.script {
        let f = temp_file(&req.cmdline).map_err(|e| failure(format!("cannot write the script file: {e}")))?;
        cmd.arg(f.path());
        Some(f)
    } else {
        cmd.arg("-c").arg(&req.cmdline);
        None
    };
    for (k, v) in &req.env {
        cmd.env(k, v);
    }
    if let Some(dir) = &req.workdir {
        cmd.current_dir(dir);
    }
    Ok((cmd, script))
}

/// What a failed start reports: the command line, or the interpreter of a
/// script.
fn describe(req: &RunRequest) -> String {
    if req.script { format!("{} script", shell_of(req)) } else { req.cmdline.clone() }
}

fn failure(msg: String) -> Outcome {
    Outcome { status: 127, stdout: String::new(), stderr: msg, timed_out: false }
}

pub fn run_interactive(req: &RunRequest) -> Outcome {
    let (mut cmd, _script) = match command(req) {
        Ok(c) => c,
        Err(out) => return out,
    };
    // The command reads and writes the terminal directly.
    if let Ok(tty) = std::fs::OpenOptions::new().read(true).write(true).open("/dev/tty")
        && let (Ok(a), Ok(b), Ok(c)) = (tty.try_clone(), tty.try_clone(), tty.try_clone()) {
            cmd.stdin(a).stdout(b).stderr(c);
        }
    match cmd.status() {
        Ok(s) => Outcome { status: exit_code(s), ..Default::default() },
        Err(e) => failure(format!("cannot run `{}`: {e}", describe(req))),
    }
}

fn exit_code(s: std::process::ExitStatus) -> i32 {
    use std::os::unix::process::ExitStatusExt;
    s.code().unwrap_or_else(|| 128 + s.signal().unwrap_or(0))
}

/// Starts `req` in its own process group with piped output. The guard
/// holds a script's temporary file until the command ends.
fn spawn_child(req: &RunRequest) -> Result<(Child, Option<TempPath>), Outcome> {
    let (mut cmd, script) = command(req)?;
    cmd.stdin(if req.stdin.is_some() { Stdio::piped() } else { Stdio::null() })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .process_group(0);
    let mut child = cmd.spawn().map_err(|e| failure(format!("cannot run `{}`: {e}", describe(req))))?;
    if let (Some(mut sin), Some(data)) = (child.stdin.take(), req.stdin.clone()) {
        // Write from a thread so a command that does not read stdin cannot
        // block us; the pipe closes when the thread ends.
        thread::spawn(move || {
            let _ = sin.write_all(data.as_bytes());
        });
    }
    Ok((child, script))
}

/// Waits for `child`. The timeout or `cancel` sends SIGTERM to its group,
/// and SIGKILL follows after `grace`. Returns the status and whether the
/// timeout fired.
fn wait_child(child: &mut Child, req: &RunRequest, grace: Duration, cancel: Option<&AtomicBool>) -> (i32, bool) {
    let pgid = child.id() as i32;
    let start = Instant::now();
    let mut timed_out = false;
    let mut term_sent: Option<Instant> = None;
    let status = loop {
        match child.try_wait() {
            Ok(Some(s)) => break exit_code(s),
            Ok(None) => {}
            Err(_) => break -1,
        }
        if term_sent.is_none() {
            let expired = req.timeout.is_some_and(|limit| start.elapsed() >= limit);
            if expired || cancel.is_some_and(|c| c.load(Ordering::SeqCst)) {
                timed_out = expired;
                signal_group(pgid, libc::SIGTERM);
                term_sent = Some(Instant::now());
            }
        }
        if term_sent.is_some_and(|t| t.elapsed() >= grace) {
            signal_group(pgid, libc::SIGKILL);
        }
        thread::sleep(Duration::from_millis(5));
    };
    if term_sent.is_some() {
        // Make sure nothing in the group outlives us and holds the pipes.
        signal_group(pgid, libc::SIGKILL);
    }
    (status, timed_out)
}

pub fn run_captured(req: &RunRequest, grace: Duration) -> Outcome {
    let (mut child, _script) = match spawn_child(req) {
        Ok(c) => c,
        Err(out) => return out,
    };
    let read_all = |r: Option<Box<dyn Read + Send>>| {
        thread::spawn(move || {
            let mut buf = Vec::new();
            if let Some(mut r) = r {
                let _ = r.read_to_end(&mut buf);
            }
            String::from_utf8_lossy(&buf).into_owned()
        })
    };
    let out_t = read_all(child.stdout.take().map(|s| Box::new(s) as Box<dyn Read + Send>));
    let err_t = read_all(child.stderr.take().map(|s| Box::new(s) as Box<dyn Read + Send>));
    let (status, timed_out) = wait_child(&mut child, req, grace, None);
    Outcome { status, stdout: out_t.join().unwrap_or_default(), stderr: err_t.join().unwrap_or_default(), timed_out }
}

/// Runs `req`, sending each output line to `events` as it arrives. The
/// returned outcome has no output; it all went out as lines.
pub fn run_streaming(req: &RunRequest, grace: Duration, id: JobId, events: &Sender<JobEvent>, cancel: &AtomicBool) -> Outcome {
    let (mut child, _script) = match spawn_child(req) {
        Ok(c) => c,
        Err(out) => {
            for l in out.stderr.lines() {
                let _ = events.send(JobEvent::Line { job: id, text: l.to_string(), stderr: true });
            }
            return Outcome { stderr: String::new(), ..out };
        }
    };
    let lines = |r: Option<Box<dyn Read + Send>>, stderr: bool| {
        let events = events.clone();
        thread::spawn(move || {
            let Some(r) = r else { return };
            let mut r = BufReader::new(r);
            let mut buf = Vec::new();
            while matches!(r.read_until(b'\n', &mut buf), Ok(n) if n > 0) {
                let text = String::from_utf8_lossy(&buf);
                let text = text.trim_end_matches(['\n', '\r']).to_string();
                if events.send(JobEvent::Line { job: id, text, stderr }).is_err() {
                    return;
                }
                buf.clear();
            }
        })
    };
    let out_t = lines(child.stdout.take().map(|s| Box::new(s) as Box<dyn Read + Send>), false);
    let err_t = lines(child.stderr.take().map(|s| Box::new(s) as Box<dyn Read + Send>), true);
    let (status, timed_out) = wait_child(&mut child, req, grace, Some(cancel));
    let _ = out_t.join();
    let _ = err_t.join();
    Outcome { status, timed_out, ..Default::default() }
}

fn signal_group(pgid: i32, sig: i32) {
    // SAFETY: kill(2) with a negative pid signals the process group we
    // created for the child; it has no memory-safety preconditions.
    unsafe {
        libc::kill(-pgid, sig);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn req(cmd: &str) -> RunRequest {
        RunRequest { cmdline: cmd.into(), ..Default::default() }
    }

    #[test]
    fn captures_stdout_stderr_and_status() {
        let out = run_captured(&req("echo out; echo err >&2; exit 3"), KILL_GRACE);
        assert_eq!(out.stdout, "out\n");
        assert_eq!(out.stderr, "err\n");
        assert_eq!(out.status, 3);
        assert!(!out.ok());
    }

    #[test]
    fn values_travel_in_environment_not_command_line() {
        let mut r = req("printf '%s' \"$TARGET\"");
        r.env.push(("TARGET".into(), "/tmp/x; rm -rf /".into()));
        let out = run_captured(&r, KILL_GRACE);
        assert_eq!(out.stdout, "/tmp/x; rm -rf /");
    }

    #[test]
    fn stdin_is_passed() {
        let mut r = req("read -r line; printf '%s' \"$line\"");
        r.stdin = Some("secret\n".into());
        assert_eq!(run_captured(&r, KILL_GRACE).stdout, "secret");
    }

    #[test]
    fn stdin_is_closed_when_not_given() {
        let out = run_captured(&req("cat; echo done"), KILL_GRACE);
        assert_eq!(out.stdout, "done\n");
    }

    #[test]
    fn workdir_is_used() {
        let mut r = req("pwd -P");
        r.workdir = Some("/".into());
        assert_eq!(run_captured(&r, KILL_GRACE).stdout, "/\n");
    }

    #[test]
    fn timeout_sends_sigterm() {
        let mut r = req("sleep 5");
        r.timeout = Some(Duration::from_millis(100));
        let t = Instant::now();
        let out = run_captured(&r, KILL_GRACE);
        assert!(out.timed_out);
        assert!(!out.ok());
        assert!(t.elapsed() < Duration::from_secs(2), "{:?}", t.elapsed());
        assert_eq!(out.status, 128 + libc::SIGTERM);
    }

    #[test]
    fn sigkill_after_grace_when_sigterm_ignored() {
        let mut r = req("trap '' TERM; while :; do sleep 0.05; done");
        r.timeout = Some(Duration::from_millis(100));
        let t = Instant::now();
        let out = run_captured(&r, Duration::from_millis(200));
        assert!(out.timed_out);
        assert!(t.elapsed() < Duration::from_secs(2), "{:?}", t.elapsed());
    }

    #[test]
    fn missing_shell_command_reports_status() {
        let out = run_captured(&req("definitely-not-a-command-xyz"), KILL_GRACE);
        assert_eq!(out.status, 127);
        assert!(out.stderr.contains("not found"));
    }

    #[test]
    fn shell_selects_the_interpreter() {
        let mut r = req("echo ${BASH_VERSION:+bash}");
        r.shell = Some("bash".into());
        assert_eq!(run_captured(&r, KILL_GRACE).stdout, "bash\n");
    }

    #[test]
    fn scripts_run_from_a_private_file_that_is_removed() {
        let mut r = req("# a comment\nprintf '%s %s\\n' \"$X\" \"$0\"\nls -l \"$0\" | cut -c1-10\nread -r line; echo \"$line\"\n");
        r.script = true;
        r.env.push(("X".into(), "x".into()));
        r.stdin = Some("from stdin\n".into());
        let out = run_captured(&r, KILL_GRACE);
        let lines: Vec<&str> = out.stdout.lines().collect();
        assert_eq!(lines.len(), 3, "{out:?}");
        let (x, path) = lines[0].split_once(' ').unwrap();
        assert_eq!(x, "x");
        assert_eq!(lines[1], "-rw-------");
        assert_eq!(lines[2], "from stdin", "stdin stays free for the command");
        assert!(!Path::new(path).exists(), "the script file is removed");
    }

    #[test]
    fn non_utf8_output_is_lossy() {
        let out = run_captured(&req("printf '\\377ok'"), KILL_GRACE);
        assert!(out.stdout.ends_with("ok"));
    }

    fn collect(rx: &std::sync::mpsc::Receiver<JobEvent>) -> (Vec<(String, bool)>, Outcome) {
        let mut lines = Vec::new();
        loop {
            match rx.recv_timeout(Duration::from_secs(5)).expect("job event") {
                JobEvent::Line { text, stderr, .. } => lines.push((text, stderr)),
                JobEvent::Done { outcome, .. } => return (lines, outcome),
                e @ JobEvent::Evaluated { .. } => panic!("unexpected {e:?}"),
            }
        }
    }

    #[test]
    fn spawn_streams_lines_then_done() {
        let (tx, rx) = std::sync::mpsc::channel();
        let mut runner = ProcessRunner::default();
        let cancel = Arc::new(AtomicBool::new(false));
        assert!(runner.spawn(&req("echo a; echo b >&2; printf c; exit 4"), 7, tx, cancel));
        let (mut lines, out) = collect(&rx);
        lines.sort();
        assert_eq!(lines, [("a".to_string(), false), ("b".to_string(), true), ("c".to_string(), false)]);
        assert_eq!(out.status, 4);
        assert!(out.stdout.is_empty());
    }

    #[test]
    fn spawn_does_not_block_and_can_be_cancelled() {
        let (tx, rx) = std::sync::mpsc::channel();
        let mut runner = ProcessRunner { handover: None, kill_grace: Some(Duration::from_millis(200)) };
        let cancel = Arc::new(AtomicBool::new(false));
        let t = Instant::now();
        assert!(runner.spawn(&req("echo started; sleep 5"), 1, tx, cancel.clone()));
        assert!(t.elapsed() < Duration::from_millis(500), "spawn returns at once");
        match rx.recv_timeout(Duration::from_secs(2)).unwrap() {
            JobEvent::Line { text, .. } => assert_eq!(text, "started"),
            e => panic!("{e:?}"),
        }
        cancel.store(true, Ordering::SeqCst);
        let (_, out) = collect(&rx);
        assert_eq!(out.status, 128 + libc::SIGTERM);
        assert!(!out.timed_out);
        assert!(t.elapsed() < Duration::from_secs(2), "{:?}", t.elapsed());
    }

    #[test]
    fn spawn_reports_a_command_that_cannot_start() {
        let (tx, rx) = std::sync::mpsc::channel();
        let mut r = req("true");
        r.workdir = Some("/definitely/not/here".into());
        assert!(ProcessRunner::default().spawn(&r, 1, tx, Arc::new(AtomicBool::new(false))));
        let (lines, out) = collect(&rx);
        assert_eq!(out.status, 127);
        assert!(lines[0].0.contains("cannot run"), "{lines:?}");
    }

    #[test]
    fn handover_called_around_interactive_runs() {
        use std::sync::Mutex;
        let calls = Arc::new(Mutex::new(Vec::new()));
        let c2 = calls.clone();
        let mut runner = ProcessRunner { handover: Some(Arc::new(move |b| c2.lock().unwrap().push(b))), kill_grace: None };
        let mut r = req("true");
        r.interactive = true;
        let out = runner.run(&r);
        assert_eq!(out.status, 0);
        assert_eq!(*calls.lock().unwrap(), [true, false]);
    }
}
