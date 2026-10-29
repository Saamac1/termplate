//! End-to-end tests through `termplate render` (spec §14.4): the real
//! binary and shell host, without a terminal.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

fn example() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/backup/app.termplate")
}

fn tmpdir(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("termplate-render-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// Runs `termplate render ARGS` with `steps` on stdin, in `dir`.
fn render(dir: &Path, args: &[&str], steps: &str) -> (i32, String, String) {
    render_env(dir, args, steps, &[])
}

/// [`render`] with extra environment variables.
fn render_env(dir: &Path, args: &[&str], steps: &str, env: &[(&str, &Path)]) -> (i32, String, String) {
    let mut child = Command::new(env!("CARGO_BIN_EXE_termplate"))
        .args(["render", "--size", "70x20", "--steps", "-"])
        .args(args)
        .current_dir(dir)
        .env("LANG", "en_US.UTF-8")
        .env("XDG_CONFIG_HOME", dir.join("no-config"))
        .envs(env.iter().copied())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(steps.as_bytes()).unwrap();
    let out = child.wait_with_output().unwrap();
    (out.status.code().unwrap_or(-1), String::from_utf8_lossy(&out.stdout).into(), String::from_utf8_lossy(&out.stderr).into())
}

fn write(dir: &Path, name: &str, src: &str) -> String {
    std::fs::write(dir.join(name), src).unwrap();
    name.to_string()
}

#[test]
fn snapshot_without_steps() {
    let out = Command::new(env!("CARGO_BIN_EXE_termplate"))
        .args(["render", "--size", "40x10", "menu"])
        .arg(example())
        .arg("MAIN_MENU")
        .env("LANG", "en_US.UTF-8")
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert_eq!(out.status.code(), Some(0), "{}", String::from_utf8_lossy(&out.stderr));
    let lines: Vec<&str> = stdout.lines().collect();
    assert_eq!(lines[0], "--- screen 40x10");
    assert_eq!(lines.len(), 11, "every row: {stdout}");
    assert!(stdout.contains("Back up") && stdout.contains("Restore"), "{stdout}");
}

#[test]
fn menu_choice_and_back() {
    let d = example().parent().unwrap().to_path_buf();
    let (code, out, err) = render(&d, &["menu", "app.termplate", "MAIN_MENU"], "key j\nexpect Restore\nkey enter\nwait\n");
    assert_eq!((code, out.as_str()), (0, "--- exit 0\nRESTORE\n"), "{err}");
    let (code, out, _) = render(&d, &["menu", "app.termplate", "MAIN_MENU"], "key esc\n");
    assert_eq!((code, out.as_str()), (1, "--- exit 1\n"), "esc is back");
}

#[test]
fn failed_expectation_exits_4_with_the_screen() {
    let d = example().parent().unwrap().to_path_buf();
    let (code, out, err) = render(&d, &["menu", "app.termplate", "MAIN_MENU"], "expect Restore\nreject Restore\n");
    assert_eq!(code, 4);
    assert!(out.is_empty());
    assert!(err.contains("step 2: `Restore` is on the screen:") && err.contains("Back up"), "{err}");
}

#[test]
fn step_errors() {
    let d = example().parent().unwrap().to_path_buf();
    let (code, _, err) = render(&d, &["menu", "app.termplate", "MAIN_MENU"], "key j\npress x\n");
    assert_eq!(code, 3);
    assert!(err.contains("line 2: unknown step"), "{err}");
    let (code, _, err) = render(&d, &["menu", "app.termplate", "MAIN_MENU"], "key esc\nscreen\n");
    assert_eq!(code, 3);
    assert!(err.contains("step 2: the program already ended"), "{err}");
}

#[test]
fn serve_events_and_messages() {
    let d = example().parent().unwrap().to_path_buf();
    let (code, out, err) = render(&d, &["serve", "app.termplate", "MAIN_MENU"], "send set TARGET /var\nsend bogus\nkey enter\nkey esc\n");
    assert_eq!(code, 0, "{err}");
    assert_eq!(out, "--- event error protocol unknown message `bogus`\n--- event select MAIN_MENU BACKUP\n--- event closed MAIN_MENU\n--- exit 0\n");
}

const SLOW_APP: &str = "app SLOW
- termplate 0.1

layout MAIN
mmll

legend:
- m MAIN_MENU
- l LOG

menu MAIN_MENU
- SLOW \"Slow\"
- QUIT \"Quit\"

command SLOW
- run echo started; sleep 5; echo finished
- output LOG

log LOG history 50
";

#[test]
fn background_command_streams_and_esc_cancels_it() {
    let d = tmpdir("slow");
    let app = write(&d, "slow.termplate", SLOW_APP);
    let start = Instant::now();
    let (code, out, err) = render(
        &d,
        &["run", &app],
        "key enter\nsleep 500ms\nexpect started\nreject finished\nkey esc\nwait\nexpect SLOW cancelled\nkey ctrl+q\n",
    );
    assert_eq!(code, 0, "{err}");
    assert_eq!(out, "--- exit 0\n");
    assert!(start.elapsed() < Duration::from_secs(4), "{:?}", start.elapsed());
}

#[test]
fn ending_stops_running_commands() {
    let d = tmpdir("stop");
    let app = write(&d, "slow.termplate", &SLOW_APP.replace("sleep 5", "sleep 7"));
    let (code, _, err) = render(&d, &["run", &app], "key enter\nsleep 200ms\n");
    assert_eq!(code, 0, "{err}");
    std::thread::sleep(Duration::from_millis(200));
    let left = Command::new("pgrep").args(["-f", "echo started; sleep 7"]).output().unwrap().stdout;
    assert!(left.is_empty(), "{}", String::from_utf8_lossy(&left));
}

const SLOW_PANEL_APP: &str = "app SLOWPANEL
- termplate 0.1

layout MAIN
ppmm

legend:
- p CLOCK
- m MAIN_MENU

panel CLOCK refresh 1s
vvvvvvvvvv

legend:
- v value $(sleep 1; echo ticked)

menu MAIN_MENU
- FIRST \"First\" open OTHER
- QUIT \"Quit\" quit

menu OTHER
- BACK \"Back\" back
";

#[test]
fn slow_panel_values_do_not_hold_up_keys() {
    let d = tmpdir("panel");
    let app = write(&d, "panel.termplate", SLOW_PANEL_APP);
    let (code, _, err) = render(&d, &["run", &app], "reject ticked\nkey enter\nexpect Back\nreject ticked\nwait\nexpect ticked\nkey ctrl+q\n");
    assert_eq!(code, 0, "{err}");
}

#[test]
fn form_widget_prints_values() {
    let d = tmpdir("form");
    let ui = write(
        &d,
        "form.termplate",
        "vars\n- HOST text \"localhost\"\n- PORT number 22 range 1..65535\n\npanel LOGIN\nhhhh [host__________]\npppp [port_]\n\nlegend:\n- h text LOGIN.HOST \"Host\"\n- p text LOGIN.PORT \"Port\"\n- host input HOST\n- port input PORT\n",
    );
    let steps = format!("{}type example.org\nkey tab backspace backspace\ntype 2222\nkey enter\n", "key backspace\n".repeat(9));
    let (code, out, err) = render(&d, &["panel", &ui, "LOGIN"], &steps);
    assert_eq!((code, out.as_str()), (0, "--- exit 0\nexample.org\n2222\n"), "{err}");
}

#[test]
fn embedded_script_output_in_the_log() {
    let d = tmpdir("script");
    let app = write(
        &d,
        "s.termplate",
        "app S\n- termplate 0.1\n\nvars\n- NAME text \"world\"\n\nlayout MAIN\nmmll\n\nlegend:\n- m MAIN_MENU\n- l LOG\n\nmenu MAIN_MENU\n- GREET \"Greet\"\n- QUIT \"Quit\"\n\ncommand GREET\n- env NAME=$NAME\n- output LOG\nrun:\n```bash\n# a comment > not a Termplate comment\necho \"hello, ${NAME}!\"\n```\n\nlog LOG history 50\n",
    );
    let (code, out, err) = render(&d, &["run", &app], "key enter\nwait\nexpect hello, world!\nkey ctrl+q\n");
    assert_eq!((code, out.as_str()), (0, "--- exit 0\n"), "{err}");
}

#[test]
fn interactive_commands_run_without_a_terminal() {
    let d = tmpdir("interactive");
    let app = write(
        &d,
        "i.termplate",
        "app I\n- termplate 0.1\n\nlayout MAIN\nmmll\n\nlegend:\n- m MAIN_MENU\n- l LOG\n\nmenu MAIN_MENU\n- VIEW \"View\"\n- QUIT \"Quit\"\n\ncommand VIEW\n- run touch viewed; echo noise interactive\n- then set DONE yes\n\nvars\n- DONE text \"no\"\n\nlog LOG\n",
    );
    let (code, out, err) = render(&d, &["run", &app], "key enter\nwait\nkey ctrl+q\n");
    assert_eq!((code, out.as_str()), (0, "--- exit 0\n"), "{err}");
    assert!(d.join("viewed").exists());
}

#[test]
fn sudoku_example() {
    let d = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/sudoku");
    let steps = "\
wait
expect 5 3 ·
expect Cells left: 51
key l l
type 4
wait
expect 5 3 4
expect Cells left: 50
type 9
wait
expect Conflict: box 1: 9
type 0
wait
expect Cells left: 51
key h h
type 1
wait
expect 5 3
reject Cells left: 50
key q
";
    let (code, out, err) = render(&d, &["run", "sudoku.termplate"], steps);
    assert_eq!((code, out.as_str()), (0, "--- exit 0\n"), "{err}");
}

#[test]
fn wizard_example() {
    let d = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/wizard");
    let tmp = tmpdir("wizard");
    // the narrow layout (70 columns): no summary, the log under the menu
    let steps = "\
wait
key w
expect Welcome
key enter
expect 1/5 Profile
type Ada
key tab
type ada@example
key enter
expect Like you@example.com
type .com
key tab
type secret
key tab
key right
key enter
expect 2/5 Options
key enter
key down
key right
key down
key enter
expect 3/5 Theme
key right
key down
key enter
wait
expect 4/5 Folder
key enter
wait
expect 5/5 Parts
key insert
key down
key insert
key enter
wait
expect Install pro (ember) for Ada?
key enter
expect Write files into the folder?
key enter
wait
expect Installed into the folder.
key down down enter
reject Installed into the folder.
expect State:     installed
key ctrl+q
";
    let (code, out, err) = render_env(&d, &["run", "app.termplate"], steps, &[("TMPDIR", &tmp)]);
    assert_eq!((code, out.as_str()), (0, "--- exit 0\n"), "{err}");
    let conf = std::fs::read_to_string(tmp.join("termplate-wizard-demo/wizard.conf")).unwrap();
    for line in ["name = Ada", "email = ada@example.com", "edition = pro", "theme = ember", "workers = 3", "verbose = true", "parts = core examples"] {
        assert!(conf.contains(line), "{line}: {conf}");
    }
    assert!(!conf.contains("secret") && !conf.contains("account = none"), "the password came on stdin and was hashed: {conf}");
}

#[test]
fn open_stack_drills_down_and_back() {
    let d = tmpdir("drill");
    std::fs::create_dir_all(d.join("a/b/c")).unwrap();
    let app = write(
        &d,
        "drill.termplate",
        "app DRILL\n- termplate 0.1\n- layout MAIN\n\nlayout MAIN grid\nMAIN_MENU\n\nmenu MAIN_MENU\n- GO open DIR(\"a\")\n- QUIT quit\n\nmenu DIR(path) list\n- source $(ls -d \"$path\"/*/ | sed 's:/$::')\n- keep\n- then open DIR($it) stack\n",
    );
    // each level keeps its own rows and argument when the one above closes
    let steps = "key enter\nwait\nexpect a/b\nkey enter\nwait\nexpect a/b/c\nkey esc\nreject a/b/c\nexpect a/b\nkey esc\nreject a/b\nexpect MAIN_MENU.GO\nkey ctrl+q\n";
    let (code, out, err) = render(&d, &["run", &app], steps);
    assert_eq!((code, out.as_str()), (0, "--- exit 0\n"), "{err}");
}
