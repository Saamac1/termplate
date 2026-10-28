//! `termplate` command line (spec §14).

use std::path::Path;
use std::process::ExitCode;

use termplate_core::engine::{Engine, Mode};
use termplate_core::loader::{dirname, load_tree, SourceFs};
use termplate_core::text::language_from_env;
use termplate_core::Model;
use termplate_shell::exec::ProcessRunner;
use termplate_shell::fs::{config_dir, override_files, DiskFs};
use termplate_shell::host::ShellHost;
use termplate_shell::runloop::{self, terminal_handover, Protocol};

/// Exit status for errors (0 = choice, 1 = back, 2 = quit; spec §14.1).
const ERROR: u8 = 3;

const USAGE: &str = "\
usage:
  termplate check FILE               validate an application
  termplate run FILE                 run the application layout
  termplate menu FILE MENU           show one menu, print the choice
  termplate input FILE INPUT         ask one input, print the value
  termplate panel FILE PANEL         show one panel
  termplate serve [--json] FILE [NAME]
                                     coprocess mode: commands on stdin,
                                     events on stdout
FILE `-` reads the Termplate file from stdin (not with serve; use <(…));
--name NAME names the application of such a file (user overrides)
exit status: 0 choice, 1 back, 2 quit, 3 error";

/// The root file as given, or one read from stdin or a pipe (`-`,
/// `<(…)`): such a file has no folder of its own, so its includes and
/// working directory are the current directory (spec §14.1).
struct Source {
    /// The name the loader sees.
    root: String,
    piped: Option<String>,
    /// `--name`.
    name: Option<String>,
}

impl Source {
    fn new(file: &str, name: Option<&str>) -> Result<Source, String> {
        let name = name.map(str::to_string);
        if file == "-" {
            let mut src = String::new();
            std::io::Read::read_to_string(&mut std::io::stdin(), &mut src).map_err(|e| format!("cannot read stdin: {e}"))?;
            return Ok(Source { root: "(stdin)".into(), piped: Some(src), name });
        }
        if let Some(fd) = file.strip_prefix("/dev/fd/").or_else(|| file.strip_prefix("/proc/self/fd/")) {
            let src = std::fs::read_to_string(file).map_err(|e| format!("cannot read {file}: {e}"))?;
            return Ok(Source { root: format!("(fd {fd})"), piped: Some(src), name });
        }
        Ok(Source { root: file.to_string(), piped: None, name })
    }
}

impl SourceFs for Source {
    fn read(&self, path: &str) -> Result<String, String> {
        match &self.piped {
            Some(src) if path == self.root => Ok(src.clone()),
            _ => DiskFs.read(path),
        }
    }
    fn glob(&self, pattern: &str) -> Result<Vec<String>, String> {
        DiskFs.glob(pattern)
    }
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match real_main(&args) {
        Ok(code) => ExitCode::from(code),
        Err(msg) => {
            eprintln!("termplate: {msg}");
            ExitCode::from(ERROR)
        }
    }
}

fn real_main(args: &[String]) -> Result<u8, String> {
    let mut argv: Vec<&str> = args.iter().map(String::as_str).collect();
    let mut name = None;
    if let Some(i) = argv.iter().position(|a| *a == "--name") {
        let n = *argv.get(i + 1).ok_or("--name needs a NAME")?;
        if !termplate_core::names::is_name(n) {
            return Err(format!("--name {n}: not a NAME (upper case, digits and _)"));
        }
        name = Some(n);
        argv.drain(i..i + 2);
    }
    match argv.as_slice() {
        [] | ["-h" | "--help" | "help"] => {
            println!("{USAGE}");
            Ok(0)
        }
        ["-V" | "--version"] => {
            println!("termplate {}", env!("CARGO_PKG_VERSION"));
            Ok(0)
        }
        ["check", file] => {
            let (model, _) = load(&Source::new(file, name)?, true)?;
            for w in &model.warnings {
                eprintln!("warning: {w}");
            }
            println!(
                "{file}: ok ({} layouts, {} panels, {} menus, {} commands, {} inputs)",
                model.layouts.len(),
                model.panels.len(),
                model.menus.len(),
                model.commands.len(),
                model.inputs.len()
            );
            Ok(0)
        }
        ["run", file] => interactive(Source::new(file, name)?, Mode::App, None, None),
        [kind @ ("menu" | "input" | "panel"), file, target] => {
            let source = Source::new(file, name)?;
            let (model, host) = load(&source, true)?;
            let exists = match *kind {
                "menu" => model.menus.contains_key(*target),
                "input" => model.inputs.contains_key(*target),
                _ => model.panels.contains_key(*target),
            };
            if !exists {
                return Err(format!("no {kind} `{target}` in {file}"));
            }
            start(model, host, Mode::Widget, Some(target), None)
        }
        ["serve", rest @ ..] => {
            let (json, rest) = match rest {
                ["--json", r @ ..] => (true, r),
                r => (false, r),
            };
            let proto = Some(if json { Protocol::Json } else { Protocol::Lines });
            if rest.first() == Some(&"-") {
                return Err("serve cannot read the Termplate file from stdin: stdin is for the protocol (use <(…))".into());
            }
            match rest {
                [file] => interactive(Source::new(file, name)?, Mode::Serve, None, proto),
                [file, target] => interactive(Source::new(file, name)?, Mode::Serve, Some(target), proto),
                _ => Err(format!("bad arguments\n{USAGE}")),
            }
        }
        _ => Err(format!("bad arguments\n{USAGE}")),
    }
}

/// Loads `source`. With `widget`, a file without an `app` block is accepted
/// (spec §2); `run` and `serve` require one.
fn load(source: &Source, widget: bool) -> Result<(Model, ShellHost<ProcessRunner>), String> {
    let mut host = ShellHost::new(ProcessRunner { handover: Some(terminal_handover()), kill_grace: None });
    let file = source.root.as_str();

    // The app name decides where the user's overrides live. A piped file
    // without an `app` block or `--name` gets none.
    let (blocks, _) = load_tree(source, file).map_err(|e| e.to_string())?;
    let app_name = blocks
        .iter()
        .find(|b| b.kind == "app")
        .map(|b| b.name.clone())
        .or_else(|| source.name.clone())
        .or_else(|| (widget && source.piped.is_none()).then(|| termplate_core::implicit_app_name(file)));
    let overrides = match &app_name {
        Some(name) => {
            let env = |k: &str| std::env::var(k).ok();
            config_dir(name, env("XDG_CONFIG_HOME").as_deref(), env("HOME").as_deref())
                .map(|d| override_files(&d))
                .unwrap_or_default()
        }
        None => vec![],
    };
    let refs: Vec<&str> = overrides.iter().map(String::as_str).collect();
    let loaded = if widget {
        termplate_core::load_widget(source, file, source.name.as_deref(), &refs, &mut host)
    } else {
        termplate_core::load(source, file, &refs, &mut host)
    };
    let mut model = loaded.map_err(|e| e.to_string())?;

    // Working directory: the application's folder unless the app says otherwise.
    let dir = dirname(file);
    let base = Path::new(if dir.is_empty() { "." } else { dir }).canonicalize().unwrap_or_else(|_| ".".into());
    let workdir = match &model.app.workdir {
        Some(w) if Path::new(w).is_absolute() => w.into(),
        Some(w) => base.join(w),
        None => base,
    };
    model.app.workdir = Some(workdir.to_string_lossy().into_owned());
    host.workdir = model.app.workdir.clone();
    if let Some(t) = model.settings.timeout {
        host.subst_timeout = t;
    }
    if model.settings.language.is_none() {
        let env_lang = ["LC_ALL", "LC_MESSAGES", "LANG"]
            .iter()
            .filter_map(|k| std::env::var(k).ok())
            .find(|v| !v.is_empty())
            .and_then(|v| language_from_env(&v));
        if let Some(lang) = env_lang {
            model.texts.language = Some(lang);
        }
    }
    Ok((model, host))
}

fn interactive(source: Source, mode: Mode, target: Option<&str>, proto: Option<Protocol>) -> Result<u8, String> {
    let (model, host) = load(&source, mode == Mode::Widget)?;
    start(model, host, mode, target, proto)
}

fn start(model: Model, mut host: ShellHost<ProcessRunner>, mode: Mode, target: Option<&str>, proto: Option<Protocol>) -> Result<u8, String> {
    let mut engine = Engine::new(model, mode);
    let (w, h) = termplate_term::terminal_size();
    engine.set_size(w, h);
    match target {
        Some(t) => engine.start_widget(&mut host, t)?,
        None => engine.start_app(&mut host)?,
    }
    let done = runloop::run(&mut engine, &mut host, proto).map_err(|e| format!("terminal: {e}"))?;
    if let Some(out) = done.output {
        println!("{out}");
    }
    Ok(done.code.clamp(0, 255) as u8)
}
