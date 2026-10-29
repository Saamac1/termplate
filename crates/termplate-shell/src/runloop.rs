//! The interactive event loop shared by all run modes (spec §14).

use std::io::{self, BufRead, Write};
use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};
use std::sync::mpsc;
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use termplate_core::engine::{Effect, Engine};
use termplate_core::protocol::{encode_event, parse_host_line, Event};
use termplate_core::Host;
use termplate_term::{map_event, no_color_from_env, terminal_size, Screen, TtyGuard};

use crate::exec::TerminalHandover;
use crate::json;

static NEED_REDRAW: AtomicBool = AtomicBool::new(false);
static MOUSE: AtomicBool = AtomicBool::new(true);
static SIGNAL: AtomicI32 = AtomicI32::new(0);

/// How often the whole screen is rewritten: the terminal can lose what we
/// drew (Cmd+K, other programs writing to it) without telling us.
const REPAINT_EVERY: Duration = Duration::from_secs(1);

extern "C" fn on_signal(sig: libc::c_int) {
    SIGNAL.store(sig, Ordering::SeqCst);
}

fn install_signal_handlers() {
    for sig in [libc::SIGTERM, libc::SIGHUP, libc::SIGINT, libc::SIGQUIT] {
        // SAFETY: the handler only stores into an atomic, which is
        // async-signal-safe.
        unsafe {
            libc::signal(sig, on_signal as *const () as libc::sighandler_t);
        }
    }
}

/// Hands the terminal to interactive commands and takes it back.
pub fn terminal_handover() -> TerminalHandover {
    Arc::new(|give: bool| {
        if give {
            TtyGuard::suspend();
        } else {
            let _ = TtyGuard::resume(MOUSE.load(Ordering::SeqCst));
            NEED_REDRAW.store(true, Ordering::SeqCst);
        }
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Protocol {
    Lines,
    Json,
}

/// How the loop ended: exit code and text for stdout (widget results).
pub struct Finished {
    pub code: i32,
    pub output: Option<String>,
}

pub fn run(engine: &mut Engine, host: &mut dyn Host, serve: Option<Protocol>) -> io::Result<Finished> {
    install_signal_handlers();
    let mouse = engine.model.settings.mouse != Some(false);
    MOUSE.store(mouse, Ordering::SeqCst);
    let (_guard, tty) = TtyGuard::new(mouse)?;
    let no_color = no_color_from_env(std::env::var("NO_COLOR").ok().as_deref());
    let mut screen = Screen::new(tty, engine.model.style.clone(), no_color);
    let (w, h) = terminal_size();
    engine.set_size(w, h);

    let rx = serve.map(|_| {
        let (tx, rx) = mpsc::channel::<Option<String>>();
        thread::spawn(move || {
            let stdin = io::stdin();
            for line in stdin.lock().lines() {
                match line {
                    Ok(l) => {
                        if tx.send(Some(l)).is_err() {
                            return;
                        }
                    }
                    Err(_) => break,
                }
            }
            let _ = tx.send(None);
        });
        rx
    });

    let result = event_loop(engine, host, serve, &mut screen, rx.as_ref());
    // Background commands do not outlive the program: ask them to stop and
    // give them a moment to get the SIGTERM.
    engine.cancel_all(host);
    let deadline = Instant::now() + Duration::from_millis(300);
    while engine.has_jobs() && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(10));
        engine.poll_jobs(host);
    }
    result
}

fn event_loop(
    engine: &mut Engine,
    host: &mut dyn Host,
    serve: Option<Protocol>,
    screen: &mut Screen<impl Write>,
    rx: Option<&mpsc::Receiver<Option<String>>>,
) -> io::Result<Finished> {
    let mut stdout = io::stdout();
    let emit = |stdout: &mut io::Stdout, e: &Event| -> io::Result<()> {
        let line = match serve {
            Some(Protocol::Json) => json::encode_event(e),
            _ => encode_event(e),
        };
        writeln!(stdout, "{line}")?;
        stdout.flush()
    };

    let mut last_repaint = Instant::now();
    loop {
        if last_repaint.elapsed() >= REPAINT_EVERY {
            screen.repaint();
            last_repaint = Instant::now();
        }
        if NEED_REDRAW.swap(false, Ordering::SeqCst) {
            screen.invalidate();
            let (w, h) = terminal_size();
            engine.set_size(w, h);
        }
        let sig = SIGNAL.load(Ordering::SeqCst);
        if sig != 0 {
            return Ok(Finished { code: 128 + sig, output: None });
        }
        for effect in engine.take_effects() {
            match effect {
                Effect::Quit => return Ok(Finished { code: 0, output: None }),
                Effect::Exit { code, output } => return Ok(Finished { code, output }),
                Effect::Event(e) if serve.is_some() => emit(&mut stdout, &e)?,
                Effect::Event(_) => {}
            }
        }
        let frame = engine.render();
        screen.draw(&frame)?;

        let mut timeout = engine.next_refresh().unwrap_or(Duration::from_secs(1));
        if engine.busy() {
            timeout = timeout.min(Duration::from_millis(120));
        }
        if engine.has_jobs() {
            timeout = timeout.min(Duration::from_millis(40));
        }
        if rx.is_some() {
            timeout = timeout.min(Duration::from_millis(30));
        }
        if crossterm::event::poll(timeout)?
            && let Some(ev) = map_event(crossterm::event::read()?) {
                engine.handle(host, ev);
            }
        if let Some(rx) = rx {
            while let Ok(msg) = rx.try_recv() {
                let Some(line) = msg else {
                    // The script closed our stdin: it is done with us.
                    return Ok(Finished { code: 0, output: None });
                };
                if line.trim().is_empty() {
                    continue;
                }
                let parsed = match serve {
                    Some(Protocol::Json) => json::parse_host_json(&line),
                    _ => parse_host_line(&line),
                };
                let result = parsed.and_then(|m| engine.apply(host, m));
                if let Err(e) = result {
                    emit(&mut stdout, &Event::new("error", "protocol", Some(&e)))?;
                }
            }
        }
        engine.poll_jobs(host);
        engine.tick(host, Instant::now());
    }
}
