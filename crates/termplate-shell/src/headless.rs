//! `termplate render`: any run mode without a terminal (spec §14.4), for
//! automated tests in place of a pseudo-terminal.
//!
//! The engine runs with the real shell host at a fixed size. Steps from a
//! file drive it, and stdout gets the screens, protocol events and the
//! result:
//!
//! ```text
//! --- screen 80x24          (`screen`: every row, trailing spaces cut)
//! --- event select M B      (serve mode: each event as it comes)
//! --- exit 0                (the program ended: its status, then its
//! RESTORE                    widget output)
//! ```
//!
//! Steps, one per line (`#` starts a comment line):
//!
//! * `key KEY…`: keys as in `keys` blocks (`j`, `enter`, `ctrl+q`).
//! * `type TEXT`: the characters of TEXT (the rest of the line).
//! * `mouse BINDING X Y`: `click`, `rclick`, `wheel.up`… at a cell.
//! * `wait [LIMIT]`: until no command or evaluation runs (default 30s).
//! * `sleep TIME`: lets time pass (panel `refresh`, streaming output).
//! * `resize WxH`, `screen`, `send MESSAGE` (serve mode: from the host).
//! * `expect TEXT`, `reject TEXT`: the screen must (not) contain TEXT.
//!
//! After the program ended only `wait` and `sleep` may follow.

use std::io::{self, Write};
use std::thread;
use std::time::{Duration, Instant};

use termplate_core::engine::{Effect, Engine, InputEvent};
use termplate_core::keys::{parse_binding, Binding};
use termplate_core::model::parse_duration;
use termplate_core::protocol::{encode_event, parse_host_line, Event};
use termplate_core::Host;

use crate::json;
use crate::runloop::Protocol;

pub const DEFAULT_SIZE: (usize, usize) = (80, 24);
const DEFAULT_WAIT: Duration = Duration::from_secs(30);

/// Exit status when an `expect` or `reject` step fails.
pub const EXPECT_FAILED: i32 = 4;

#[derive(Debug, Clone, PartialEq)]
pub enum Step {
    Input(Vec<InputEvent>),
    Wait(Duration),
    Sleep(Duration),
    Resize(usize, usize),
    Screen,
    Send(String),
    Expect { text: String, present: bool },
}

/// `80x24`.
pub fn parse_size(s: &str) -> Option<(usize, usize)> {
    let (w, h) = s.split_once('x')?;
    let (w, h) = (w.parse().ok()?, h.parse().ok()?);
    (w > 0 && h > 0).then_some((w, h))
}

fn duration(s: &str) -> Result<Duration, String> {
    parse_duration(s).ok_or_else(|| format!("bad duration `{s}` (e.g. 200ms, 2s)"))
}

/// Parses a steps file; errors name the line.
pub fn parse_steps(src: &str) -> Result<Vec<(usize, Step)>, String> {
    let mut steps = Vec::new();
    for (i, line) in src.lines().enumerate() {
        let n = i + 1;
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let (word, rest) = line.split_once(' ').map(|(w, r)| (w, r.trim())).unwrap_or((line, ""));
        let args: Vec<&str> = rest.split_whitespace().collect();
        let step = match (word, args.as_slice()) {
            ("key", [_, ..]) => {
                let mut events = Vec::new();
                for k in &args {
                    match parse_binding(k) {
                        Ok(Binding::Key(code, mods)) => events.push(InputEvent::Key(code, mods)),
                        Ok(_) => return Err(format!("line {n}: `{k}` is not a key (use `mouse`)")),
                        Err(e) => return Err(format!("line {n}: {e}")),
                    }
                }
                Step::Input(events)
            }
            ("type", _) if !rest.is_empty() => Step::Input(rest.chars().map(InputEvent::ch).collect()),
            ("mouse", [b, x, y]) => {
                let (Ok(x), Ok(y)) = (x.parse(), y.parse()) else {
                    return Err(format!("line {n}: `mouse BINDING X Y`"));
                };
                match parse_binding(b) {
                    Ok(binding @ Binding::Mouse(..)) => Step::Input(vec![InputEvent::Mouse { binding, x, y }]),
                    _ => return Err(format!("line {n}: `{b}` is not a mouse binding (click, rclick, wheel.up…)")),
                }
            }
            ("wait", []) => Step::Wait(DEFAULT_WAIT),
            ("wait", [t]) => Step::Wait(duration(t).map_err(|e| format!("line {n}: {e}"))?),
            ("sleep", [t]) => Step::Sleep(duration(t).map_err(|e| format!("line {n}: {e}"))?),
            ("resize", [s]) => {
                let (w, h) = parse_size(s).ok_or_else(|| format!("line {n}: `resize WxH`"))?;
                Step::Resize(w, h)
            }
            ("screen", []) => Step::Screen,
            ("send", _) if !rest.is_empty() => Step::Send(rest.to_string()),
            ("expect" | "reject", _) if !rest.is_empty() => Step::Expect { text: rest.to_string(), present: word == "expect" },
            _ => {
                return Err(format!(
                    "line {n}: unknown step `{line}` (key, type, mouse, wait, sleep, resize, screen, send, expect, reject)"
                ));
            }
        };
        steps.push((n, step));
    }
    Ok(steps)
}

/// The steps used without a steps file: settle, then show the screen.
pub fn default_steps() -> Vec<(usize, Step)> {
    vec![(0, Step::Wait(DEFAULT_WAIT)), (0, Step::Screen)]
}

struct Run<'a, W: Write> {
    engine: &'a mut Engine,
    host: &'a mut dyn Host,
    serve: Option<Protocol>,
    out: W,
    /// Exit code and output once the program ended.
    finished: Option<(i32, Option<String>)>,
}

impl<W: Write> Run<'_, W> {
    fn emit(&mut self, e: &Event) -> io::Result<()> {
        let line = match self.serve {
            Some(Protocol::Json) => json::encode_event(e),
            _ => encode_event(e),
        };
        writeln!(self.out, "--- event {line}")
    }

    /// Takes the engine's effects, like the terminal loop does.
    fn effects(&mut self) -> io::Result<()> {
        for effect in self.engine.take_effects() {
            match effect {
                Effect::Quit => self.finished = self.finished.take().or(Some((0, None))),
                Effect::Exit { code, output } => self.finished = self.finished.take().or(Some((code, output))),
                Effect::Event(e) if self.serve.is_some() => self.emit(&e)?,
                Effect::Event(_) => {}
            }
        }
        Ok(())
    }

    fn settle(&mut self) -> io::Result<()> {
        self.engine.poll_jobs(self.host);
        self.engine.tick(self.host, Instant::now());
        self.effects()
    }

    fn screen(&mut self) -> Vec<String> {
        self.engine.render().rows().into_iter().map(|r| r.trim_end().to_string()).collect()
    }

    fn step(&mut self, step: &Step) -> Result<(), String> {
        let io = |e: io::Error| e.to_string();
        match step {
            Step::Input(events) => {
                for ev in events {
                    self.engine.handle(self.host, ev.clone());
                    self.effects().map_err(io)?;
                    if self.finished.is_some() {
                        break;
                    }
                }
            }
            Step::Wait(limit) => {
                let start = Instant::now();
                loop {
                    self.settle().map_err(io)?;
                    if self.finished.is_some() || !self.engine.has_jobs() {
                        break;
                    }
                    if start.elapsed() >= *limit {
                        return Err(format!("still running after {limit:?}"));
                    }
                    thread::sleep(Duration::from_millis(5));
                }
            }
            Step::Sleep(t) => {
                let end = Instant::now() + *t;
                while Instant::now() < end && self.finished.is_none() {
                    self.settle().map_err(io)?;
                    thread::sleep(Duration::from_millis(5).min(end.saturating_duration_since(Instant::now())));
                }
                self.settle().map_err(io)?;
            }
            Step::Resize(w, h) => self.engine.set_size(*w, *h),
            Step::Screen => {
                let (w, h) = self.engine.size();
                let rows = self.screen();
                writeln!(self.out, "--- screen {w}x{h}").map_err(io)?;
                for r in rows {
                    writeln!(self.out, "{r}").map_err(io)?;
                }
            }
            Step::Send(line) => {
                let parsed = match self.serve {
                    None => return Err("`send` needs `serve`".into()),
                    Some(Protocol::Json) => json::parse_host_json(line),
                    Some(Protocol::Lines) => parse_host_line(line),
                };
                if let Err(e) = parsed.and_then(|m| self.engine.apply(self.host, m)) {
                    self.emit(&Event::new("error", "protocol", Some(&e))).map_err(io)?;
                }
                self.effects().map_err(io)?;
            }
            Step::Expect { text, present } => {
                let rows = self.screen();
                if rows.iter().any(|r| r.contains(text.as_str())) != *present {
                    let what = if *present { "not on the screen" } else { "on the screen" };
                    return Err(format!("`{text}` is {what}:\n{}", rows.join("\n")));
                }
            }
        }
        Ok(())
    }
}

/// How a headless run ended.
#[derive(Debug, PartialEq)]
pub enum Outcome {
    /// The exit status for the process.
    Done(i32),
    /// A step failed (line, message); `expect` failures exit with
    /// [`EXPECT_FAILED`], others are errors.
    Failed { line: usize, message: String, expect: bool },
}

/// Runs `steps` on a started engine, writing to `out`.
pub fn run(engine: &mut Engine, host: &mut dyn Host, serve: Option<Protocol>, steps: &[(usize, Step)], out: impl Write) -> io::Result<Outcome> {
    let mut r = Run { engine, host, serve, out, finished: None };
    r.effects()?;
    let mut result = Outcome::Done(0);
    for (line, step) in steps {
        if r.finished.is_some() {
            // Waiting after the end is harmless; anything else is a mistake.
            if matches!(step, Step::Wait(_) | Step::Sleep(_)) {
                continue;
            }
            result = Outcome::Failed { line: *line, message: "the program already ended".into(), expect: false };
            break;
        }
        if let Err(message) = r.step(step) {
            result = Outcome::Failed { line: *line, message, expect: matches!(step, Step::Expect { .. }) };
            break;
        }
    }
    // Background commands do not outlive the run.
    r.engine.cancel_all(r.host);
    let deadline = Instant::now() + Duration::from_millis(300);
    while r.engine.has_jobs() && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(10));
        r.engine.poll_jobs(r.host);
    }
    if let Some((code, output)) = r.finished.take() {
        writeln!(r.out, "--- exit {code}")?;
        if let Some(o) = output {
            writeln!(r.out, "{o}")?;
        }
        if result == Outcome::Done(0) {
            result = Outcome::Done(code);
        }
    }
    r.out.flush()?;
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use termplate_core::keys::{KeyCode, Mods};

    #[test]
    fn steps_parse() {
        let s = parse_steps("# comment\n\nkey j ctrl+q enter\ntype a b\nmouse click 3 4\nwait\nwait 2s\nsleep 200ms\nresize 100x30\nscreen\nsend set A 1\nexpect Hello world\nreject Error\n").unwrap();
        let ctrl = Mods { ctrl: true, ..Default::default() };
        assert_eq!(s[0], (3, Step::Input(vec![InputEvent::ch('j'), InputEvent::Key(KeyCode::Char('q'), ctrl), InputEvent::key(KeyCode::Enter)])));
        assert_eq!(s[1].1, Step::Input(vec![InputEvent::ch('a'), InputEvent::ch(' '), InputEvent::ch('b')]));
        assert!(matches!(&s[2].1, Step::Input(v) if matches!(v[0], InputEvent::Mouse { x: 3, y: 4, .. })));
        assert_eq!(s[3].1, Step::Wait(DEFAULT_WAIT));
        assert_eq!(s[4].1, Step::Wait(Duration::from_secs(2)));
        assert_eq!(s[5].1, Step::Sleep(Duration::from_millis(200)));
        assert_eq!(s[6].1, Step::Resize(100, 30));
        assert_eq!(s[7].1, Step::Screen);
        assert_eq!(s[8].1, Step::Send("set A 1".into()));
        assert_eq!(s[9].1, Step::Expect { text: "Hello world".into(), present: true });
        assert_eq!(s[10].1, Step::Expect { text: "Error".into(), present: false });
    }

    #[test]
    fn step_errors_name_the_line() {
        assert!(parse_steps("screen\nkey hyper+k\n").unwrap_err().starts_with("line 2:"));
        assert!(parse_steps("key click\n").unwrap_err().contains("not a key"));
        assert!(parse_steps("mouse j 1 2\n").unwrap_err().contains("not a mouse binding"));
        assert!(parse_steps("wait soon\n").unwrap_err().contains("bad duration"));
        assert!(parse_steps("press j\n").unwrap_err().contains("unknown step"));
        assert!(parse_steps("expect\n").unwrap_err().contains("unknown step"));
    }

    #[test]
    fn sizes() {
        assert_eq!(parse_size("80x24"), Some((80, 24)));
        assert_eq!(parse_size("0x24"), None);
        assert_eq!(parse_size("80"), None);
    }
}
