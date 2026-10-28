//! `- window` commands (spec §15.1): the command runs in a window of its
//! own and Termplate does not wait for it.
//!
//! * In tmux (`TMUX`): `new-window -d` (tab), `split-window` next to
//!   Termplate's pane (split), `display-popup -E` (popup, tmux 3.2+;
//!   older ones get a tab).
//! * Without tmux: a Terminal window on macOS (`osascript`), `$TERMINAL`
//!   or `x-terminal-emulator` under a graphical session on Linux.
//! * Nowhere to open a window: the command runs `interactive` in this
//!   terminal, with a warning.
//!
//! The new window does not inherit our environment, and values must not
//! show up in `ps`. So a private folder (700) gets the environment as an
//! `env` file (600), the `run:` script, and a wrapper that reads and
//! removes the environment, runs the command and removes the folder. Only
//! the wrapper's path goes on the window's command line.

use std::io::{Read, Write};
use std::os::unix::fs::OpenOptionsExt;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use termplate_core::{Outcome, RunRequest, Window};

use crate::exec::{shell_of, temp_dir};
use crate::fs::{find_program, shell_quote};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Opener {
    /// `popup` works (tmux 3.2+).
    Tmux { popup: bool, pane: Option<String> },
    MacTerminal,
    /// A terminal program taking `-e PROGRAM ARGS…`.
    Terminal(String),
}

/// Where a window can be opened, from the environment (`env`), the
/// programs found (`has`) and whether this is macOS.
pub fn choose(env: &dyn Fn(&str) -> Option<String>, has: &dyn Fn(&str) -> bool, macos: bool, tmux_version: &dyn Fn() -> Option<String>) -> Option<Opener> {
    let set = |k: &str| env(k).is_some_and(|v| !v.is_empty());
    if set("TMUX") && has("tmux") {
        let popup = tmux_version().as_deref().and_then(parse_version).is_some_and(|v| v >= (3, 2));
        return Some(Opener::Tmux { popup, pane: env("TMUX_PANE").filter(|p| !p.is_empty()) });
    }
    let remote = set("SSH_CONNECTION") || set("SSH_TTY");
    if macos {
        return (!remote && has("osascript")).then_some(Opener::MacTerminal);
    }
    if !set("DISPLAY") && !set("WAYLAND_DISPLAY") {
        return None;
    }
    env("TERMINAL").filter(|t| !t.is_empty() && has(t)).or_else(|| has("x-terminal-emulator").then(|| "x-terminal-emulator".to_string())).map(Opener::Terminal)
}

/// `tmux 3.3a` → (3, 3); `tmux next-3.4` → (3, 4).
fn parse_version(s: &str) -> Option<(u32, u32)> {
    let v = s.split_whitespace().nth(1)?;
    let v = v.trim_start_matches(|c: char| !c.is_ascii_digit());
    let mut parts = v.split('.');
    let major = parts.next()?.parse().ok()?;
    let minor: String = parts.next().unwrap_or("0").chars().take_while(char::is_ascii_digit).collect();
    Some((major, minor.parse().unwrap_or(0)))
}

/// The program that opens the window running `cmd` (a `sh` command line).
pub fn launcher(opener: &Opener, kind: Window, cmd: &str, workdir: Option<&str>) -> Vec<String> {
    let s = |v: &[&str]| v.iter().map(|x| x.to_string()).collect::<Vec<_>>();
    match opener {
        Opener::Tmux { popup, pane } => {
            let mut argv = match kind {
                Window::Popup if *popup => s(&["tmux", "display-popup", "-E"]),
                Window::Split => {
                    let mut a = s(&["tmux", "split-window"]);
                    if let Some(p) = pane {
                        a.extend(["-t".to_string(), p.clone()]);
                    }
                    a
                }
                _ => s(&["tmux", "new-window", "-d"]),
            };
            if let Some(dir) = workdir {
                // display-popup names its folder with -d
                let flag = if argv[1] == "display-popup" { "-d" } else { "-c" };
                argv.extend([flag.to_string(), dir.to_string()]);
            }
            argv.push(cmd.to_string());
            argv
        }
        Opener::MacTerminal => {
            let script = cmd.replace('\\', "\\\\").replace('"', "\\\"");
            s(&["osascript", "-e", "tell application \"Terminal\"", "-e", "activate", "-e", &format!("do script \"{script}\""), "-e", "end tell"])
        }
        Opener::Terminal(t) => s(&[t, "-e", "sh", "-c", cmd]),
    }
}

/// `export NAME='value'` lines; names a shell cannot export are skipped.
pub fn env_file(vars: &[(String, String)]) -> String {
    let mut out = String::new();
    for (k, v) in vars {
        let ok = k.starts_with(|c: char| c.is_ascii_alphabetic() || c == '_') && k.chars().all(|c| c.is_ascii_alphanumeric() || c == '_');
        if ok {
            out.push_str(&format!("export {k}={}\n", shell_quote(v)));
        }
    }
    out
}

/// Our environment for the window: everything but what belongs to this
/// terminal or shell, then the command's own entries.
fn window_env(req: &RunRequest) -> Vec<(String, String)> {
    const SKIP: &[&str] = &["TMUX_PANE", "SHLVL", "PWD", "OLDPWD", "_", "COLUMNS", "LINES"];
    let mut env: Vec<(String, String)> = std::env::vars().filter(|(k, _)| !SKIP.contains(&k.as_str())).collect();
    for (k, v) in &req.env {
        env.retain(|(ek, _)| ek != k);
        env.push((k.clone(), v.clone()));
    }
    env
}

/// The wrapper run in the window, inside folder `dir`.
pub fn wrapper(req: &RunRequest, dir: &Path) -> String {
    let d = shell_quote(&dir.to_string_lossy());
    let shell = shell_quote(shell_of(req));
    let run = if req.script { format!("{shell} {d}/script") } else { format!("{shell} -c {}", shell_quote(&req.cmdline)) };
    let cd = req.workdir.as_deref().map(|w| format!("cd {} 2>/dev/null\n", shell_quote(w))).unwrap_or_default();
    format!(". {d}/env\nrm -f {d}/env\n{cd}{run}\nstatus=$?\nrm -rf {d}\nexit $status\n")
}

fn write_private(path: &Path, content: &str) -> std::io::Result<()> {
    let mut f = std::fs::OpenOptions::new().write(true).create_new(true).mode(0o600).open(path)?;
    f.write_all(content.as_bytes())
}

/// Opens `req` in a window; `fallback` runs it interactively here when no
/// window can be opened.
pub fn open(req: &RunRequest, kind: Window, fallback: &dyn Fn(&RunRequest) -> Outcome) -> Outcome {
    let path = std::env::var("PATH").ok();
    let has = |p: &str| find_program(p, path.as_deref()).is_some();
    let version = || Command::new("tmux").arg("-V").output().ok().map(|o| String::from_utf8_lossy(&o.stdout).into_owned());
    let Some(opener) = choose(&|k| std::env::var(k).ok(), &has, cfg!(target_os = "macos"), &version) else {
        let out = fallback(&RunRequest { window: None, interactive: true, ..req.clone() });
        let warning = "no window could be opened (no tmux and no graphical terminal); the command ran in this terminal";
        return Outcome { stderr: format!("{warning}\n{}", out.stderr), ..out };
    };
    match launch(req, kind, &opener) {
        Ok(()) => Outcome::default(),
        Err(msg) => Outcome { status: 1, stderr: format!("cannot open a window: {msg}"), ..Default::default() },
    }
}

fn launch(req: &RunRequest, kind: Window, opener: &Opener) -> Result<(), String> {
    let dir = temp_dir().map_err(|e| e.to_string())?;
    let io = |e: std::io::Error| e.to_string();
    write_private(&dir.path().join("env"), &env_file(&window_env(req))).map_err(io)?;
    if req.script {
        write_private(&dir.path().join("script"), &req.cmdline).map_err(io)?;
    }
    let run = dir.path().join("run");
    write_private(&run, &wrapper(req, dir.path())).map_err(io)?;
    let argv = launcher(opener, kind, &format!("sh {}", shell_quote(&run.to_string_lossy())), req.workdir.as_deref());
    let mut child = Command::new(&argv[0])
        .args(&argv[1..])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("{}: {e}", argv[0]))?;
    // The launcher returns at once once the window is up; one still running
    // (a popup, a terminal program that stays) counts as opened.
    let patience = if kind == Window::Popup { Duration::from_millis(300) } else { Duration::from_secs(3) };
    let start = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(status)) if status.success() => break,
            Ok(Some(_)) => {
                let mut err = String::new();
                if let Some(mut e) = child.stderr.take() {
                    let _ = e.read_to_string(&mut err);
                }
                return Err(format!("{} failed: {}", argv[0], err.trim()));
            }
            Ok(None) if start.elapsed() >= patience => {
                std::thread::spawn(move || child.wait());
                break;
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(10)),
            Err(e) => return Err(e.to_string()),
        }
    }
    // The wrapper removes the folder when the command ends.
    dir.keep();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    fn env_of(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> {
        let m: BTreeMap<String, String> = pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
        move |k| m.get(k).cloned()
    }

    #[test]
    fn chooses_tmux_first() {
        let env = env_of(&[("TMUX", "/tmp/tmux-1/default,1,0"), ("TMUX_PANE", "%3"), ("DISPLAY", ":0")]);
        let o = choose(&env, &|_| true, true, &|| Some("tmux 3.3a\n".into()));
        assert_eq!(o, Some(Opener::Tmux { popup: true, pane: Some("%3".into()) }));
        let o = choose(&env, &|_| true, false, &|| Some("tmux 3.1c".into()));
        assert_eq!(o, Some(Opener::Tmux { popup: false, pane: Some("%3".into()) }));
        let o = choose(&env, &|p| p != "tmux", false, &|| None);
        assert_eq!(o, Some(Opener::Terminal("x-terminal-emulator".into())), "TMUX set but no tmux program");
    }

    #[test]
    fn chooses_a_graphical_terminal_or_nothing() {
        assert_eq!(choose(&env_of(&[]), &|_| true, true, &|| None), Some(Opener::MacTerminal));
        assert_eq!(choose(&env_of(&[("SSH_TTY", "/dev/ttys001")]), &|_| true, true, &|| None), None, "SSH without tmux");
        assert_eq!(choose(&env_of(&[]), &|_| true, false, &|| None), None, "Linux console");
        let env = env_of(&[("WAYLAND_DISPLAY", "wayland-0"), ("TERMINAL", "foot")]);
        assert_eq!(choose(&env, &|_| true, false, &|| None), Some(Opener::Terminal("foot".into())));
        assert_eq!(choose(&env, &|p| p == "x-terminal-emulator", false, &|| None), Some(Opener::Terminal("x-terminal-emulator".into())));
        assert_eq!(choose(&env, &|_| false, false, &|| None), None);
    }

    #[test]
    fn tmux_versions() {
        assert_eq!(parse_version("tmux 3.2"), Some((3, 2)));
        assert_eq!(parse_version("tmux 3.3a"), Some((3, 3)));
        assert_eq!(parse_version("tmux next-3.4"), Some((3, 4)));
        assert_eq!(parse_version("tmux master"), None);
    }

    #[test]
    fn launchers() {
        let t = Opener::Tmux { popup: true, pane: Some("%1".into()) };
        assert_eq!(launcher(&t, Window::Tab, "sh '/t/run'", Some("/w")), ["tmux", "new-window", "-d", "-c", "/w", "sh '/t/run'"]);
        assert_eq!(launcher(&t, Window::Split, "C", None), ["tmux", "split-window", "-t", "%1", "C"]);
        assert_eq!(launcher(&t, Window::Popup, "C", Some("/w")), ["tmux", "display-popup", "-E", "-d", "/w", "C"]);
        let old = Opener::Tmux { popup: false, pane: None };
        assert_eq!(launcher(&old, Window::Popup, "C", None), ["tmux", "new-window", "-d", "C"], "old tmux: a tab");
        let mac = launcher(&Opener::MacTerminal, Window::Split, "sh '/t/r\"un'", None);
        assert_eq!(mac[6], "do script \"sh '/t/r\\\"un'\"");
        assert_eq!(launcher(&Opener::Terminal("foot".into()), Window::Tab, "C", None), ["foot", "-e", "sh", "-c", "C"]);
    }

    #[test]
    fn env_file_quotes_values_and_skips_bad_names() {
        let f = env_file(&[("A".into(), "it's; $(x)".into()), ("BAD-NAME".into(), "x".into()), ("_B1".into(), "".into())]);
        assert_eq!(f, "export A='it'\\''s; $(x)'\nexport _B1=''\n");
    }

    #[test]
    fn wrapper_reads_env_runs_and_cleans_up() {
        let dir = temp_dir().unwrap();
        let req = RunRequest {
            cmdline: "printf '%s|%s' \"$X\" \"$(pwd -P)\"; ls \"$D\"; exit 4".into(),
            env: vec![("X".into(), "a 'b'".into())],
            workdir: Some("/".into()),
            ..Default::default()
        };
        let mut env = req.env.clone();
        env.push(("D".into(), dir.path().to_string_lossy().into_owned()));
        write_private(&dir.path().join("env"), &env_file(&env)).unwrap();
        let run = dir.path().join("run");
        write_private(&run, &wrapper(&req, dir.path())).unwrap();
        let out = Command::new("sh").arg(&run).env_remove("X").output().unwrap();
        assert_eq!(out.status.code(), Some(4));
        let stdout = String::from_utf8_lossy(&out.stdout);
        assert_eq!(stdout, "a 'b'|/run\n", "env read and removed before the command");
        assert!(!dir.path().exists(), "the folder is removed");
        dir.keep();
    }

    #[test]
    fn wrapper_runs_scripts_with_the_interpreter() {
        let dir = temp_dir().unwrap();
        let req = RunRequest { cmdline: "echo ${BASH_VERSION:+bash}\n".into(), script: true, shell: Some("bash".into()), ..Default::default() };
        write_private(&dir.path().join("env"), "").unwrap();
        write_private(&dir.path().join("script"), &req.cmdline).unwrap();
        let run = dir.path().join("run");
        write_private(&run, &wrapper(&req, dir.path())).unwrap();
        let out = Command::new("sh").arg(&run).output().unwrap();
        assert_eq!(String::from_utf8_lossy(&out.stdout), "bash\n");
        assert!(!dir.path().exists());
        dir.keep();
    }
}
