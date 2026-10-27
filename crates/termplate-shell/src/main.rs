//! `termplate` command line (spec §14).

use std::path::Path;
use std::process::ExitCode;

use termplate_core::engine::{Engine, Mode};
use termplate_core::loader::{dirname, load_tree};
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
exit status: 0 choice, 1 back, 2 quit, 3 error";

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
    let argv: Vec<&str> = args.iter().map(String::as_str).collect();
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
            let (model, _) = load(file, true)?;
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
        ["run", file] => interactive(file, Mode::App, None, None),
        [kind @ ("menu" | "input" | "panel"), file, name] => {
            let (model, _) = load(file, true)?;
            let exists = match *kind {
                "menu" => model.menus.contains_key(*name),
                "input" => model.inputs.contains_key(*name),
                _ => model.panels.contains_key(*name),
            };
            if !exists {
                return Err(format!("no {kind} `{name}` in {file}"));
            }
            interactive(file, Mode::Widget, Some(name), None)
        }
        ["serve", rest @ ..] => {
            let (json, rest) = match rest {
                ["--json", r @ ..] => (true, r),
                r => (false, r),
            };
            let proto = Some(if json { Protocol::Json } else { Protocol::Lines });
            match rest {
                [file] => interactive(file, Mode::Serve, None, proto),
                [file, name] => interactive(file, Mode::Serve, Some(name), proto),
                _ => Err(format!("bad arguments\n{USAGE}")),
            }
        }
        _ => Err(format!("bad arguments\n{USAGE}")),
    }
}

/// Loads `file`. With `widget`, a file without an `app` block is accepted
/// (spec §2); `run` and `serve` require one.
fn load(file: &str, widget: bool) -> Result<(Model, ShellHost<ProcessRunner>), String> {
    let mut host = ShellHost::new(ProcessRunner { handover: Some(terminal_handover()), kill_grace: None });

    // The app name decides where the user's overrides live.
    let (blocks, _) = load_tree(&DiskFs, file).map_err(|e| e.to_string())?;
    let app_name = blocks
        .iter()
        .find(|b| b.kind == "app")
        .map(|b| b.name.clone())
        .or_else(|| widget.then(|| termplate_core::implicit_app_name(file)));
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
        termplate_core::load_widget(&DiskFs, file, &refs, &mut host)
    } else {
        termplate_core::load(&DiskFs, file, &refs, &mut host)
    };
    let mut model = loaded.map_err(|e| e.to_string())?;

    // Working directory: the application's folder unless the app says otherwise.
    let base = Path::new(dirname(file)).canonicalize().unwrap_or_else(|_| ".".into());
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

fn interactive(file: &str, mode: Mode, target: Option<&str>, proto: Option<Protocol>) -> Result<u8, String> {
    let (model, mut host) = load(file, mode == Mode::Widget)?;
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
