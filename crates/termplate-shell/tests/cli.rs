//! CLI tests for the non-interactive paths (`check`, usage, errors, user
//! overrides). Interactive paths are covered by `scripts/pty_smoke.py`.

use std::path::{Path, PathBuf};
use std::process::Command;

fn bin() -> Command {
    Command::new(env!("CARGO_BIN_EXE_termplate"))
}

fn example() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/backup/app.termplate")
}

fn tmpdir(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("termplate-cli-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn run(cmd: &mut Command) -> (i32, String, String) {
    let out = cmd.output().unwrap();
    (out.status.code().unwrap_or(-1), String::from_utf8_lossy(&out.stdout).into(), String::from_utf8_lossy(&out.stderr).into())
}

#[test]
fn check_accepts_the_example_app() {
    let (code, out, err) = run(bin().arg("check").arg(example()).env("XDG_CONFIG_HOME", tmpdir("empty")));
    assert_eq!(code, 0, "{err}");
    assert!(out.contains("ok (1 layouts, 1 panels, 3 menus, 2 commands, 1 inputs)"), "{out}");
}

#[test]
fn check_reports_errors_with_location_and_status_3() {
    let d = tmpdir("broken");
    let f = d.join("app.termplate");
    std::fs::write(&f, "app X\n- termplate 0.1\nmenu M\n- NOPE\nkeys MENU\n- up hyper+k\n").unwrap();
    let (code, out, err) = run(bin().arg("check").arg(&f));
    assert_eq!(code, 3);
    assert!(out.is_empty());
    assert!(err.contains("app.termplate:4: `NOPE` has no action and there is no command `NOPE`"), "{err}");
    assert!(err.contains("app.termplate:6: unknown modifier"), "{err}");
}

#[test]
fn missing_file_is_an_error() {
    let (code, _, err) = run(bin().args(["check", "/definitely/not/here.termplate"]));
    assert_eq!(code, 3);
    assert!(err.contains("cannot read"), "{err}");
}

#[test]
fn usage_and_version() {
    let (code, out, _) = run(&mut bin());
    assert_eq!(code, 0);
    assert!(out.contains("usage:"));
    let (code, out, _) = run(bin().arg("--version"));
    assert_eq!(code, 0);
    assert!(out.starts_with("termplate "));
    let (code, _, err) = run(bin().args(["frobnicate"]));
    assert_eq!(code, 3);
    assert!(err.contains("bad arguments"));
}

#[test]
fn widget_with_unknown_name_fails_before_touching_the_terminal() {
    let (code, _, err) = run(bin().arg("menu").arg(example()).arg("NOPE"));
    assert_eq!(code, 3);
    assert!(err.contains("no menu `NOPE`"), "{err}");
}

#[test]
fn user_overrides_from_xdg_config_are_applied() {
    let cfg = tmpdir("xdg");
    let dir = cfg.join("termplate/BACKUPTOOL/overrides");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("keys.termplate"), "keys MAIN\n- BACKUP x\n").unwrap();
    let (code, _, err) = run(bin().arg("check").arg(example()).env("XDG_CONFIG_HOME", &cfg));
    assert_eq!(code, 0, "{err}");

    // A typo in an override is a load error (spec §13).
    std::fs::write(dir.join("keys.termplate"), "keys MAIN\n- BACKUPP x\n").unwrap();
    let (code, _, err) = run(bin().arg("check").arg(example()).env("XDG_CONFIG_HOME", &cfg));
    assert_eq!(code, 3);
    assert!(err.contains("keys.termplate:2: cannot patch `BACKUPP`"), "{err}");
}

#[test]
fn user_settings_file_is_loaded() {
    let cfg = tmpdir("settings");
    let dir = cfg.join("termplate/BACKUPTOOL");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("settings.termplate"), "settings\n- volume 11\n").unwrap();
    let (code, _, err) = run(bin().arg("check").arg(example()).env("XDG_CONFIG_HOME", &cfg));
    assert_eq!(code, 3);
    assert!(err.contains("unknown setting `volume`"), "{err}");
}
