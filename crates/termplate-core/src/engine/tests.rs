use super::*;
use crate::host::{Expr, Outcome, TestHost};
use crate::loader::{merge, MergeMode};
use crate::model::Column;
use crate::protocol::parse_host_line;
use crate::syntax::parse;
use std::time::Duration;

const APP: &str = r#"app BACKUPTOOL
- termplate 0.1
- layout MAIN
- language fi

vars
- TARGET text "/mnt/backup"
- VERBOSE bool false
- LEVEL number 3 range 1..9
- POINTS number 2
- USED number 40
- OUT text
- PASS text
- FILES list
- MODE choice fast safe

layout MAIN
sssmm
lllll

legend:
- s STATUS size.width 20
- m MAIN_MENU
- l LOG size.height 4

layout MAIN.narrow when width < 40
m
l

legend:
- m MAIN_MENU
- l LOG size.height 3

panel STATUS refresh 2s
TTTTTTTTTT
dddd pppp
[when____]

legend:
- T text STATUS.TITLE
- d value $TARGET
- p bar $USED of 100
- when value $(date)

panel DONE(file)
Done: wwwwwwww

legend:
- w param file

panel HELP
hhhhhhhhhh
hhhhhhhhhh

legend:
- h text HELP.BODY

menu MAIN_MENU
- BACKUP
- RESTORE disabled when not $VERBOSE
- SETTINGS open SETTINGS
- QUIT

menu SETTINGS context POPUP
- VERBOSE toggle VERBOSE
- LEVEL adjust LEVEL 1..9 pool POINTS
- BACK

menu FILE_LIST list
- source $FILES
- sort $it desc
- keep
- then run "view $IT"

menu PICK list
- source $FILES
- filter
- multi

menu SIZES(dir) list
- source $FILES
- row NAME: $it, DIR: $dir

menu GAME_OVER final
- AGAIN open MAIN_MENU
- QUIT

command BACKUP
- run ./backup.sh
- env TARGET=$TARGET
- output LOG

command RESTORE
- confirm RESTORE.CONFIRM
- run ./restore.sh
- output LOG
- on_error open DONE(file: $TARGET)

command WHOAMI
- run whoami
- into OUT

command SHOW
- run ./show.sh

command LOGIN
- run ./login.sh
- secret PASS stdin

input TARGET_DIR
- var TARGET
- kind path
- validate $VALUE == "/ok"
- error TARGET_DIR.INVALID

input PASSWORD
- var PASS
- kind password

input LEVEL_IN
- var LEVEL
- kind number

input MODE_IN
- var MODE
- kind choice

log LOG history 50

keys MAIN
- BACKUP b

keys POPUP passthrough
- back q

keys ALWAYS
- open GAME_OVER f9
- toggle STATUS f2

text fi
STATUS.TITLE Tila
MAIN_MENU.BACKUP Varmuuskopioi
MAIN_MENU.RESTORE Palauta
MAIN_MENU.SETTINGS Asetukset
MAIN_MENU.QUIT Lopeta
RESTORE.CONFIRM Palautetaanko {TARGET}?
TARGET_DIR.INVALID Hakemistoa ei ole
SETTINGS.TITLE Asetukset
SIZES.ROW {NAME} in {DIR}
HELP Paina {^confirm}
HELP.BODY one two three four five six seven
"#;

fn model_of(src: &str) -> Model {
    let blocks: Vec<_> = parse("app.termplate", src).unwrap_or_else(|e| panic!("{e:?}")).blocks().cloned().collect();
    let (blocks, _) = merge(vec![], blocks, MergeMode::Base).unwrap();
    Model::build(blocks, &mut TestHost::new()).unwrap_or_else(|e| panic!("{e}"))
}

fn model() -> Model {
    model_of(APP)
}

fn app_with(m: Model) -> (Engine, TestHost) {
    let mut host = TestHost::new();
    let mut e = Engine::new(m, Mode::App);
    e.set_size(60, 12);
    e.start_app(&mut host).unwrap();
    (e, host)
}

fn app() -> (Engine, TestHost) {
    app_with(model())
}

fn widget(target: &str) -> (Engine, TestHost) {
    let mut host = TestHost::new();
    let mut e = Engine::new(model(), Mode::Widget);
    e.set_size(60, 20);
    e.start_widget(&mut host, target).unwrap();
    (e, host)
}

fn key(e: &mut Engine, h: &mut TestHost, k: KeyCode) {
    e.handle(h, InputEvent::key(k));
}

fn typ(e: &mut Engine, h: &mut TestHost, s: &str) {
    for c in s.chars() {
        e.handle(h, InputEvent::ch(c));
    }
}

fn cursor_key(e: &Engine, menu: &str) -> String {
    e.menu_state(menu).unwrap().current().unwrap().key.clone()
}

fn screen(e: &mut Engine) -> String {
    e.render().rows().join("\n")
}

fn stack_names(e: &Engine) -> Vec<String> {
    e.context_stack().into_iter().map(|s| s.name).collect()
}

fn ctrl(c: char) -> InputEvent {
    InputEvent::Key(KeyCode::Char(c), Mods { ctrl: true, ..Default::default() })
}

// ----- app mode: layout, panels, menus --------------------------------------

#[test]
fn app_starts_with_focus_on_first_focusable_area() {
    let (e, _) = app();
    assert_eq!(e.focused_target().as_deref(), Some("MAIN_MENU"));
    assert_eq!(stack_names(&e), ["MAIN", "MENU"]);
    assert!(e.context_stack()[1].passthrough, "embedded areas pass keys through");
}

#[test]
fn app_renders_layout_panel_and_menu() {
    let (mut e, _) = app();
    let f = e.render();
    let rows = f.rows();
    assert!(rows[0].starts_with("Tila"), "{rows:?}");
    assert!(rows[1].starts_with("/mn… ██░░"), "4-cell field is cut with …, bar 40%: {rows:?}");
    assert!(rows[0].contains("Varmuuskopioi"));
    let x = rows[0].chars().position(|c| c == 'V').unwrap();
    assert_eq!(f.get(x, 0).unwrap().role, Role::Selected);
    assert_eq!(f.area_of(f.get(x, 0).unwrap()), ["MENU".to_string(), "MAIN_MENU".to_string()], "style chain");
    let x = rows[1].chars().position(|c| c == 'P').unwrap();
    assert_eq!(f.get(x, 1).unwrap().role, Role::Disabled);
}

#[test]
fn narrow_terminal_uses_variant_layout() {
    let (mut e, _) = app();
    e.set_size(30, 12);
    assert!(e.render().row(0).starts_with("Varmuuskopioi"));
}

#[test]
fn too_small_terminal_shows_warning() {
    let (mut e, _) = app();
    e.set_size(22, 3);
    let f = e.render();
    assert!(f.row(0).starts_with("Terminal too small"), "{:?}", f.row(0));
    assert_eq!(f.get(0, 0).unwrap().role, Role::Warning);
}

#[test]
fn cursor_skips_disabled_and_reenables_after_var_change() {
    let (mut e, mut h) = app();
    typ(&mut e, &mut h, "j");
    assert_eq!(cursor_key(&e, "MAIN_MENU"), "SETTINGS", "RESTORE is disabled; j is down by default");
    e.set_var("VERBOSE", Value::Bool(true));
    e.handle(&mut h, InputEvent::Resize(60, 12));
    typ(&mut e, &mut h, "k");
    assert_eq!(cursor_key(&e, "MAIN_MENU"), "RESTORE");
}

#[test]
fn submenu_toggle_adjust_with_pool_and_back() {
    let (mut e, mut h) = app();
    key(&mut e, &mut h, KeyCode::Down);
    key(&mut e, &mut h, KeyCode::Enter);
    assert_eq!(e.top().as_deref(), Some("SETTINGS"));
    assert_eq!(stack_names(&e), ["MAIN", "MENU", "POPUP"]);
    typ(&mut e, &mut h, " "); // space confirms by default: toggle VERBOSE
    assert_eq!(e.var("VERBOSE"), Some(&Value::Bool(true)));
    key(&mut e, &mut h, KeyCode::Down); // LEVEL, passthrough to MENU
    key(&mut e, &mut h, KeyCode::Right);
    key(&mut e, &mut h, KeyCode::Right);
    assert_eq!(e.var("LEVEL"), Some(&Value::Number(5.0)));
    assert_eq!(e.var("POINTS"), Some(&Value::Number(0.0)), "paid from the pool");
    key(&mut e, &mut h, KeyCode::Right);
    assert_eq!(e.var("LEVEL"), Some(&Value::Number(5.0)), "pool is empty");
    key(&mut e, &mut h, KeyCode::Left);
    assert_eq!(e.var("POINTS"), Some(&Value::Number(1.0)), "lowering returns points");
    typ(&mut e, &mut h, "q"); // POPUP binds q to back
    assert_eq!(e.top(), None);
}

#[test]
fn modal_menu_is_drawn_with_title_on_border() {
    let (mut e, mut h) = app();
    key(&mut e, &mut h, KeyCode::Down);
    key(&mut e, &mut h, KeyCode::Enter);
    let s = screen(&mut e);
    assert!(s.contains("┌─ Asetukset ─"), "{s}");
    assert!(s.contains("│ SETTINGS.VERBOSE"), "missing translations show keys: {s}");
}

#[test]
fn styled_borders_for_modals_and_areas() {
    let src = APP.replace("log LOG history 50", "log LOG history 50\n\npanels\n- POPUP border double\n- LOG border ascii");
    let (mut e, mut h) = app_with(model_of(&src));
    let s = screen(&mut e);
    assert!(s.contains("+----"), "ascii border around the log area: {s}");
    key(&mut e, &mut h, KeyCode::Down);
    key(&mut e, &mut h, KeyCode::Enter);
    assert!(screen(&mut e).contains("╔═ Asetukset ═"), "POPUP context style");
}

#[test]
fn command_runs_with_env_and_output_goes_to_log() {
    let (mut e, mut h) = app();
    h.run_results.insert("./backup.sh".into(), Outcome { status: 0, stdout: "copied 3 files\n".into(), stderr: "warn\n".into(), timed_out: false });
    key(&mut e, &mut h, KeyCode::Enter);
    assert_eq!(h.runs[0].cmdline, "./backup.sh");
    assert!(h.runs[0].env.contains(&("TARGET".into(), "/mnt/backup".into())));
    // Only `env` entries reach the command (spec §6).
    assert!(!h.runs[0].env.iter().any(|(k, _)| k == "LEVEL" || k == "VERBOSE"), "{:?}", h.runs[0].env);
    assert_eq!(e.log_lines("LOG"), ["copied 3 files", "warn"]);
    let f = e.render();
    let y = (0..f.height).find(|&y| f.row(y).starts_with("warn")).unwrap();
    assert_eq!(f.get(0, y).unwrap().role, Role::Stderr);
}

#[test]
fn key_bound_command_in_base_context_through_embedded_menu() {
    let (mut e, mut h) = app();
    typ(&mut e, &mut h, "b");
    assert_eq!(h.runs.len(), 1);
    assert_eq!(h.runs[0].cmdline, "./backup.sh");
}

#[test]
fn modal_menu_blocks_base_context_keys() {
    let (mut e, mut h) = app();
    e.perform_named(&mut h, "open GAME_OVER");
    typ(&mut e, &mut h, "b");
    assert!(h.runs.is_empty(), "MENU is modal: `b` from MAIN does not fire");
}

#[test]
fn confirm_dialog_before_command() {
    let (mut e, mut h) = app();
    e.run_command(&mut h, "RESTORE", None);
    assert_eq!(e.top().as_deref(), Some("confirm:RESTORE"));
    let s = screen(&mut e);
    assert!(s.contains("Palautetaanko /mnt/backup?"), "{s}");
    assert!(s.contains("Enter OK  Esc Cancel"), "{s}");
    key(&mut e, &mut h, KeyCode::Esc);
    assert!(h.runs.is_empty());
    e.run_command(&mut h, "RESTORE", None);
    key(&mut e, &mut h, KeyCode::Enter);
    assert_eq!(h.runs.len(), 1);
}

#[test]
fn on_error_opens_panel_with_named_argument() {
    let (mut e, mut h) = app();
    h.run_results.insert("./restore.sh".into(), Outcome { status: 3, ..Default::default() });
    e.run_command(&mut h, "RESTORE", None);
    key(&mut e, &mut h, KeyCode::Enter);
    assert_eq!(e.top().as_deref(), Some("DONE"));
    assert!(screen(&mut e).contains("Done: /mnt/ba…"));
    // a modal panel uses MENU bindings: Enter closes it
    key(&mut e, &mut h, KeyCode::Enter);
    assert_eq!(e.top(), None);
}

#[test]
fn failed_command_and_timeout_are_reported() {
    let (mut e, mut h) = app();
    h.run_results.insert("./backup.sh".into(), Outcome { status: 2, ..Default::default() });
    key(&mut e, &mut h, KeyCode::Enter);
    assert_eq!(e.log_lines("LOG"), ["Command failed with status 2"]);
    h.run_results.insert("./backup.sh".into(), Outcome { status: -1, timed_out: true, ..Default::default() });
    key(&mut e, &mut h, KeyCode::Enter);
    assert_eq!(e.log_lines("LOG").last().unwrap(), "Command timed out");
}

#[test]
fn into_stores_stdout_without_trailing_newline() {
    let (mut e, mut h) = app();
    h.run_results.insert("whoami".into(), Outcome { stdout: "seppo\n".into(), ..Default::default() });
    e.run_command(&mut h, "WHOAMI", None);
    assert_eq!(e.var("OUT"), Some(&Value::Text("seppo".into())));
    assert_eq!(e.top(), None, "no output modal for `into`");
}

#[test]
fn output_without_area_opens_modal() {
    let (mut e, mut h) = app();
    h.run_results.insert("./show.sh".into(), Outcome { stdout: "hello\n".into(), ..Default::default() });
    e.run_command(&mut h, "SHOW", None);
    assert_eq!(e.top().as_deref(), Some("output"));
    assert!(screen(&mut e).contains("│ hello"));
    key(&mut e, &mut h, KeyCode::Enter);
    assert_eq!(e.top(), None);
}

#[test]
fn secrets_are_not_in_env_but_on_stdin() {
    let (mut e, mut h) = app();
    e.set_var("PASS", Value::Text("hunter2".into()));
    e.run_command(&mut h, "LOGIN", None);
    assert_eq!(h.runs[0].stdin.as_deref(), Some("hunter2"));
    assert!(!h.runs[0].env.iter().any(|(k, _)| k == "PASS"));
}

#[test]
fn quit_produces_effect() {
    let (mut e, mut h) = app();
    e.handle(&mut h, ctrl('q'));
    assert_eq!(e.take_effects(), [Effect::Quit]);
}

#[test]
fn quit_item_without_action_is_the_quit_action() {
    let (mut e, mut h) = app();
    key(&mut e, &mut h, KeyCode::End);
    key(&mut e, &mut h, KeyCode::Enter);
    assert_eq!(e.take_effects(), [Effect::Quit]);
}

#[test]
fn next_area_cycles_focus() {
    let (mut e, mut h) = app();
    key(&mut e, &mut h, KeyCode::Tab);
    assert_eq!(e.focused_target().as_deref(), Some("LOG"));
    assert_eq!(stack_names(&e), ["MAIN", "LOG"]);
    key(&mut e, &mut h, KeyCode::Tab);
    assert_eq!(e.focused_target().as_deref(), Some("MAIN_MENU"));
    key(&mut e, &mut h, KeyCode::BackTab);
    assert_eq!(e.focused_target().as_deref(), Some("LOG"));
}

#[test]
fn panel_refresh_follows_interval() {
    let mut h = TestHost::new();
    h.outputs.insert("$(date)".into(), "one".into());
    let mut e = Engine::new(model(), Mode::App);
    e.set_size(60, 12);
    let t0 = Instant::now();
    e.tick(&mut h, t0);
    e.start_app(&mut h).unwrap();
    assert!(screen(&mut e).contains("one"));
    h.outputs.insert("$(date)".into(), "two".into());
    e.tick(&mut h, t0 + Duration::from_secs(1));
    assert!(screen(&mut e).contains("one"), "not yet due");
    assert_eq!(e.next_refresh(), Some(Duration::from_secs(1)));
    e.tick(&mut h, t0 + Duration::from_secs(2));
    assert!(screen(&mut e).contains("two"));
}

#[test]
fn toggle_panel_hides_its_area() {
    let (mut e, mut h) = app();
    assert!(screen(&mut e).starts_with("Tila"));
    e.handle(&mut h, InputEvent::key(KeyCode::F(2)));
    assert!(e.panel_hidden("STATUS"));
    assert!(screen(&mut e).starts_with("          "), "hidden panel leaves its area empty");
    e.handle(&mut h, InputEvent::key(KeyCode::F(2)));
    assert!(screen(&mut e).starts_with("Tila"));
}

#[test]
fn multi_line_panel_field_wraps_and_cuts() {
    let (mut e, mut h) = app();
    e.perform(&mut h, &Action::Open { target: "HELP".into(), args: vec![] }, None);
    let s = screen(&mut e);
    assert!(s.contains("one two"), "{s}");
    assert!(s.contains("three …") || s.contains("three four …") || s.contains("…"), "{s}");
}

// ----- final menus and toggling with the opening key ----------------------

#[test]
fn final_menu_cannot_be_closed_with_back() {
    let (mut e, mut h) = app();
    e.handle(&mut h, InputEvent::key(KeyCode::F(9)));
    assert_eq!(e.top().as_deref(), Some("GAME_OVER"));
    key(&mut e, &mut h, KeyCode::Esc);
    assert_eq!(e.top().as_deref(), Some("GAME_OVER"));
    e.perform(&mut h, &Action::Close, None);
    assert_eq!(e.top().as_deref(), Some("GAME_OVER"));
    // its own `open` item leaves it
    key(&mut e, &mut h, KeyCode::Enter);
    assert_eq!(e.top().as_deref(), Some("MAIN_MENU"));
}

#[test]
fn always_open_binding_closes_the_menu_again() {
    let src = APP.replace("- open GAME_OVER f9", "- open SETTINGS f9");
    let (mut e, mut h) = app_with(model_of(&src));
    e.handle(&mut h, InputEvent::key(KeyCode::F(9)));
    assert_eq!(e.top().as_deref(), Some("SETTINGS"));
    e.handle(&mut h, InputEvent::key(KeyCode::F(9)));
    assert_eq!(e.top(), None);
}

// ----- logs ---------------------------------------------------------------

#[test]
fn log_history_and_scroll() {
    let (mut e, mut h) = app();
    e.render();
    for i in 0..60 {
        e.log("LOG", &format!("line {i}"), false);
    }
    assert_eq!(e.log_lines("LOG").len(), 50, "history cap");
    assert!(screen(&mut e).ends_with(&format!("{:<60}", "line 59")));
    key(&mut e, &mut h, KeyCode::Tab);
    key(&mut e, &mut h, KeyCode::Up);
    assert!(screen(&mut e).ends_with(&format!("{:<60}", "line 58")));
}

#[test]
fn log_merge_counts_repeats() {
    let src = APP.replace("log LOG history 50", "log LOG history 50 merge");
    let (mut e, _) = app_with(model_of(&src));
    e.log("LOG", "tick\ntick\ntick\ntock", false);
    assert_eq!(e.log_lines("LOG"), ["tick", "tock"]);
    assert!(screen(&mut e).contains("tick (×3)"));
}

#[test]
fn long_log_message_wraps_and_is_cut() {
    let (mut e, _) = app();
    e.set_size(20, 8);
    let words = "alpha beta gamma delta epsilon zeta eta theta iota kappa lambda mu nu xi omicron pi";
    e.log("LOG", words, false);
    let f = e.render();
    // the narrow variant has a 3-row log at the bottom
    let log_rows: Vec<String> = (5..8).map(|y| f.row(y).trim_end().to_string()).collect();
    assert_eq!(log_rows[0], "alpha beta gamma");
    assert!(log_rows[2].ends_with('…'), "{log_rows:?}");
}

#[test]
fn log_more_pages_with_any_key() {
    let src = APP.replace("log LOG history 50", "log LOG history 50 more");
    let (mut e, mut h) = app_with(model_of(&src));
    e.render();
    e.log("LOG", &(0..10).map(|i| format!("l{i}")).collect::<Vec<_>>().join("\n"), false);
    let rows = e.render().rows();
    assert_eq!(rows[8].trim_end(), "l0");
    assert_eq!(rows[11].trim_end(), "--More--");
    typ(&mut e, &mut h, "b"); // any key: next page, not a command
    assert!(h.runs.is_empty());
    assert_eq!(e.render().row(8).trim_end(), "l3");
    key(&mut e, &mut h, KeyCode::Esc); // back skips the rest
    assert_eq!(e.render().row(11).trim_end(), "l9");
}

#[test]
fn log_more_prompt_uses_log_text_key() {
    let src = APP.replace("log LOG history 50", "log LOG history 50 more").replace("HELP Paina", "LOG.MORE --Lisää--\nHELP Paina");
    let (mut e, _) = app_with(model_of(&src));
    e.render();
    e.log("LOG", &(0..10).map(|i| format!("l{i}")).collect::<Vec<_>>().join("\n"), false);
    assert_eq!(e.render().row(11).trim_end(), "--Lisää--");
}

#[test]
fn ansi_codes_are_stripped_in_logs() {
    let (mut e, _) = app();
    e.log("LOG", "\x1b[31mred\x1b[0m text", false);
    assert_eq!(e.log_lines("LOG"), ["red text"]);
}

#[test]
fn texts_resolve_key_placeholders() {
    let (e, _) = app();
    assert_eq!(e.text("HELP", &BTreeMap::new()), "Paina Enter");
}

// ----- mouse ---------------------------------------------------------------

#[test]
fn click_selects_and_confirms_menu_row_without_bindings() {
    let (mut e, mut h) = app();
    let f = e.render();
    let y = (0..f.height).find(|&y| f.row(y).contains("Asetukset")).unwrap();
    e.handle(&mut h, InputEvent::click(25, y));
    assert_eq!(e.top().as_deref(), Some("SETTINGS"));
}

#[test]
fn click_outside_modal_menu_is_back() {
    let (mut e, mut h) = app();
    e.perform_named(&mut h, "open SETTINGS");
    e.render();
    e.handle(&mut h, InputEvent::click(0, 11));
    assert_eq!(e.top(), None);
}

#[test]
fn click_on_disabled_row_does_nothing() {
    let (mut e, mut h) = app();
    e.render();
    e.handle(&mut h, InputEvent::click(25, 1));
    assert_eq!(e.top(), None);
    assert!(h.runs.is_empty());
}

#[test]
fn hover_highlights_and_wheel_scrolls() {
    let (mut e, mut h) = app();
    e.render();
    e.handle(&mut h, InputEvent::MouseMove { x: 25, y: 2 });
    assert_eq!(cursor_key(&e, "MAIN_MENU"), "SETTINGS");
    e.handle(&mut h, InputEvent::Mouse { binding: Binding::mouse(Mouse::WheelDown), x: 25, y: 0 });
    assert_eq!(cursor_key(&e, "MAIN_MENU"), "QUIT");
    e.handle(&mut h, InputEvent::click(2, 10));
    assert_eq!(e.focused_target().as_deref(), Some("LOG"));
}

#[test]
fn click_on_modal_panel_confirms() {
    let (mut e, mut h) = app();
    e.perform(&mut h, &Action::Open { target: "HELP".into(), args: vec![] }, None);
    e.render();
    e.handle(&mut h, InputEvent::click(30, 6));
    assert_eq!(e.top(), None);
}

#[test]
fn mouse_off_setting_ignores_the_mouse() {
    let src = APP.replace("log LOG history 50", "log LOG history 50\n\nsettings\n- mouse off");
    let (mut e, mut h) = app_with(model_of(&src));
    e.render();
    e.handle(&mut h, InputEvent::click(25, 2));
    assert_eq!(e.top(), None);
}

// ----- widget mode ----------------------------------------------------------

#[test]
fn widget_menu_returns_item_key() {
    let (mut e, mut h) = widget("MAIN_MENU");
    key(&mut e, &mut h, KeyCode::Enter);
    assert_eq!(e.take_effects(), [Effect::Exit { code: 0, output: Some("BACKUP".into()) }]);
    assert!(h.runs.is_empty(), "widget mode does not run commands");
}

#[test]
fn widget_back_and_quit_codes() {
    let (mut e, mut h) = widget("MAIN_MENU");
    key(&mut e, &mut h, KeyCode::Backspace);
    assert_eq!(e.take_effects(), [Effect::Exit { code: 1, output: None }], "backspace is back by default");
    let (mut e, mut h) = widget("MAIN_MENU");
    e.handle(&mut h, ctrl('q'));
    assert_eq!(e.take_effects(), [Effect::Exit { code: 2, output: None }]);
    let (mut e, mut h) = widget("MAIN_MENU");
    key(&mut e, &mut h, KeyCode::End);
    key(&mut e, &mut h, KeyCode::Enter);
    assert_eq!(e.take_effects(), [Effect::Exit { code: 2, output: None }], "QUIT item");
}

#[test]
fn widget_submenu_navigation_stays_inside() {
    let (mut e, mut h) = widget("MAIN_MENU");
    key(&mut e, &mut h, KeyCode::Down);
    key(&mut e, &mut h, KeyCode::Enter);
    assert_eq!(e.top().as_deref(), Some("SETTINGS"));
    key(&mut e, &mut h, KeyCode::Esc);
    assert_eq!(e.top().as_deref(), Some("MAIN_MENU"));
    assert!(e.take_effects().is_empty());
}

fn widget_with_files(target: &str, files: &[&str]) -> (Engine, TestHost) {
    let mut h = TestHost::new();
    let mut e = Engine::new(model(), Mode::Widget);
    e.set_size(60, 20);
    e.set_var("FILES", Value::List(files.iter().map(|s| s.to_string()).collect()));
    e.start_widget(&mut h, target).unwrap();
    (e, h)
}

#[test]
fn widget_list_sorted_with_letters() {
    let (mut e, mut h) = widget_with_files("FILE_LIST", &["b.log", "c.log", "a.log"]);
    let keys: Vec<String> = e.menu_state("FILE_LIST").unwrap().rows().map(|r| r.key.clone()).collect();
    assert_eq!(keys, ["c.log", "b.log", "a.log"], "sort desc");
    let s = screen(&mut e);
    assert!(s.contains("a c.log") && s.contains("b b.log"), "{s}");
    typ(&mut e, &mut h, "b");
    assert_eq!(e.take_effects(), [Effect::Exit { code: 0, output: Some("b.log".into()) }]);
}

#[test]
fn widget_list_filter_and_multi() {
    let (mut e, mut h) = widget_with_files("PICK", &["auth.log", "syslog", "kern.log"]);
    typ(&mut e, &mut h, "kl");
    assert_eq!(e.menu_state("PICK").unwrap().len(), 1);
    assert!(screen(&mut e).contains("/kl"));
    key(&mut e, &mut h, KeyCode::Insert);
    key(&mut e, &mut h, KeyCode::Backspace);
    key(&mut e, &mut h, KeyCode::Backspace);
    key(&mut e, &mut h, KeyCode::Home);
    key(&mut e, &mut h, KeyCode::Insert);
    assert!(screen(&mut e).contains("[x] auth.log"));
    key(&mut e, &mut h, KeyCode::Enter);
    assert_eq!(e.take_effects(), [Effect::Exit { code: 0, output: Some("auth.log\nkern.log".into()) }]);
}

#[test]
fn back_clears_filter_first() {
    let (mut e, mut h) = widget_with_files("PICK", &["x"]);
    typ(&mut e, &mut h, "zz");
    key(&mut e, &mut h, KeyCode::Esc);
    assert_eq!(e.menu_state("PICK").unwrap().filter, "");
    assert!(e.take_effects().is_empty());
    key(&mut e, &mut h, KeyCode::Esc);
    assert_eq!(e.take_effects(), [Effect::Exit { code: 1, output: None }]);
}

#[test]
fn row_values_fill_the_row_text_key() {
    let mut h = TestHost::new();
    let mut e = Engine::new(model(), Mode::App);
    e.set_size(60, 12);
    e.start_app(&mut h).unwrap();
    e.set_var("FILES", Value::List(vec!["a.log".into()]));
    let dir = Expr::bare("\"/var/log\"");
    e.perform(&mut h, &Action::Open { target: "SIZES".into(), args: vec![(None, dir)] }, None);
    assert!(screen(&mut e).contains("a.log in /var/log"), "menu parameter and ROW text");
}

#[test]
fn empty_list_without_empty_text_is_not_opened() {
    let (mut e, _) = widget("FILE_LIST");
    assert_eq!(e.take_effects(), [Effect::Exit { code: 1, output: None }]);
    assert_eq!(e.top(), None);
    let src = APP.replace("HELP Paina", "FILE_LIST.EMPTY Ei tiedostoja\nHELP Paina");
    let mut h = TestHost::new();
    let mut e = Engine::new(model_of(&src), Mode::Widget);
    e.start_widget(&mut h, "FILE_LIST").unwrap();
    assert!(screen(&mut e).contains("Ei tiedostoja"));
}

#[test]
fn widget_input_edit_validate_and_return() {
    let (mut e, mut h) = widget("TARGET_DIR");
    assert!(screen(&mut e).contains("/mnt/backup"), "starts with current value");
    for _ in 0.."/mnt/backup".len() {
        key(&mut e, &mut h, KeyCode::Backspace);
    }
    typ(&mut e, &mut h, "/bad");
    key(&mut e, &mut h, KeyCode::Enter);
    assert!(e.take_effects().is_empty());
    assert!(screen(&mut e).contains("Hakemistoa ei ole"));
    key(&mut e, &mut h, KeyCode::Home);
    for _ in 0..4 {
        key(&mut e, &mut h, KeyCode::Delete);
    }
    typ(&mut e, &mut h, "/ok");
    key(&mut e, &mut h, KeyCode::Enter);
    assert_eq!(e.take_effects(), [Effect::Exit { code: 0, output: Some("/ok".into()) }]);
}

#[test]
fn input_cursor_is_reported_in_frame() {
    let (mut e, mut h) = widget("TARGET_DIR");
    key(&mut e, &mut h, KeyCode::Home);
    let f = e.render();
    let (x, y) = f.cursor.unwrap();
    assert_eq!(f.get(x, y).unwrap().ch, '/');
}

#[test]
fn password_input_is_masked_and_starts_empty() {
    let (mut e, mut h) = widget("PASSWORD");
    typ(&mut e, &mut h, "abc");
    let s = screen(&mut e);
    assert!(s.contains("***") && !s.contains("abc"));
}

#[test]
fn number_input_rejects_letters_and_checks_range() {
    let (mut e, mut h) = widget("LEVEL_IN");
    key(&mut e, &mut h, KeyCode::Backspace);
    typ(&mut e, &mut h, "x12");
    key(&mut e, &mut h, KeyCode::Enter);
    assert!(e.take_effects().is_empty());
    assert!(screen(&mut e).contains("12 is outside 1..9"));
    key(&mut e, &mut h, KeyCode::Backspace);
    key(&mut e, &mut h, KeyCode::Enter);
    assert_eq!(e.take_effects(), [Effect::Exit { code: 0, output: Some("1".into()) }]);
}

#[test]
fn choice_input_cycles_options() {
    let (mut e, mut h) = widget("MODE_IN");
    key(&mut e, &mut h, KeyCode::Right);
    key(&mut e, &mut h, KeyCode::Enter);
    assert_eq!(e.take_effects(), [Effect::Exit { code: 0, output: Some("safe".into()) }]);
}

#[test]
fn unknown_widget_target_is_error() {
    let mut e = Engine::new(model(), Mode::Widget);
    assert!(e.start_widget(&mut TestHost::new(), "NOPE").is_err());
}

#[test]
fn list_then_run_gets_it_and_keep_keeps_menu() {
    let (mut e, mut h) = app();
    e.set_var("FILES", Value::List(vec!["x.log".into()]));
    e.perform_named(&mut h, "open FILE_LIST");
    key(&mut e, &mut h, KeyCode::Enter);
    assert_eq!(h.runs[0].cmdline, "view $IT");
    assert!(h.runs[0].env.contains(&("IT".into(), "x.log".into())));
    assert_eq!(e.top().as_deref(), Some("FILE_LIST"), "keep");
}

// ----- serve mode ------------------------------------------------------------

fn serve() -> (Engine, TestHost) {
    let mut host = TestHost::new();
    let mut e = Engine::new(model(), Mode::Serve);
    e.set_size(60, 20);
    e.start_widget(&mut host, "MAIN_MENU").unwrap();
    (e, host)
}

#[test]
fn serve_select_emits_event_without_running() {
    let (mut e, mut h) = serve();
    key(&mut e, &mut h, KeyCode::Enter);
    assert_eq!(e.take_effects(), [Effect::Event(Event::new("select", "MAIN_MENU", Some("BACKUP")))]);
    assert!(h.runs.is_empty());
}

#[test]
fn serve_builtin_actions_still_run() {
    let (mut e, mut h) = serve();
    key(&mut e, &mut h, KeyCode::Down);
    key(&mut e, &mut h, KeyCode::Enter);
    assert_eq!(e.top().as_deref(), Some("SETTINGS"));
    assert_eq!(e.take_effects(), [Effect::Event(Event::new("select", "MAIN_MENU", Some("SETTINGS")))]);
    key(&mut e, &mut h, KeyCode::Esc);
    assert_eq!(e.take_effects(), [Effect::Event(Event::new("closed", "SETTINGS", None))]);
}

#[test]
fn serve_root_back_emits_closed_and_quits() {
    let (mut e, mut h) = serve();
    key(&mut e, &mut h, KeyCode::Esc);
    assert_eq!(e.take_effects(), [Effect::Event(Event::new("closed", "MAIN_MENU", None)), Effect::Quit]);
}

#[test]
fn serve_messages_from_host() {
    let (mut e, mut h) = serve();
    e.apply(&mut h, parse_host_line("set TARGET /srv").unwrap()).unwrap();
    assert_eq!(e.var("TARGET"), Some(&Value::Text("/srv".into())));
    assert!(e.apply(&mut h, parse_host_line("set NOPE x").unwrap()).is_err());
    assert!(e.apply(&mut h, parse_host_line("set LEVEL 99").unwrap()).is_err());
    e.apply(&mut h, parse_host_line("log hello\\nworld").unwrap()).unwrap();
    e.apply(&mut h, parse_host_line("log LOG third").unwrap()).unwrap();
    assert_eq!(e.log_lines("LOG"), ["hello", "world", "third"]);
    e.apply(&mut h, parse_host_line("open SETTINGS").unwrap()).unwrap();
    assert_eq!(e.top().as_deref(), Some("SETTINGS"));
    e.apply(&mut h, parse_host_line("close SETTINGS").unwrap()).unwrap();
    assert_eq!(e.top().as_deref(), Some("MAIN_MENU"));
    assert!(e.apply(&mut h, parse_host_line("close SETTINGS").unwrap()).is_err());
    e.apply(&mut h, parse_host_line("busy on").unwrap()).unwrap();
    assert!(e.busy());
    e.apply(&mut h, parse_host_line("quit").unwrap()).unwrap();
    assert_eq!(e.take_effects().last(), Some(&Effect::Quit));
}

#[test]
fn serve_key_bound_command_emits_key_event() {
    let mut h = TestHost::new();
    let mut e = Engine::new(model(), Mode::Serve);
    e.set_size(60, 12);
    e.start_app(&mut h).unwrap();
    typ(&mut e, &mut h, "b");
    assert_eq!(e.take_effects(), [Effect::Event(Event::new("key", "BACKUP", None))]);
    assert!(h.runs.is_empty());
}

#[test]
fn serve_password_input_event_hides_value() {
    let (mut e, mut h) = serve();
    e.apply(&mut h, HostMsg::Open("PASSWORD".into())).unwrap();
    typ(&mut e, &mut h, "s3cret");
    key(&mut e, &mut h, KeyCode::Enter);
    assert_eq!(e.take_effects(), [Effect::Event(Event::new("input", "PASSWORD", None))]);
    assert_eq!(e.var("PASS"), Some(&Value::Text("s3cret".into())));
}

#[test]
fn emit_action_produces_event() {
    let (mut e, mut h) = app();
    e.perform(&mut h, &Action::Emit("PING".into()), None);
    assert_eq!(e.take_effects(), [Effect::Event(Event::new("emit", "PING", None))]);
}

// ----- helpers --------------------------------------------------------------------

#[test]
fn column_width_distribution() {
    let cols = vec![
        Column { name: "A".into(), width: None, align: Align::Left },
        Column { name: "B".into(), width: Some(8), align: Align::Right },
        Column { name: "C".into(), width: None, align: Align::Left },
    ];
    assert_eq!(column_widths(&cols, 30), [10, 8, 10]);
    assert_eq!(column_widths(&cols, 31), [10, 8, 11]);
    assert_eq!(column_widths(&cols, 5), [0, 8, 0]);
}

#[test]
fn value_comparison_is_numeric_when_possible() {
    use std::cmp::Ordering::*;
    assert_eq!(compare_values("9", "10"), Less);
    assert_eq!(compare_values("b", "a"), Greater);
}

#[test]
fn ansi_stripping() {
    assert_eq!(strip_ansi("\x1b[1;32mok\x1b[0m"), "ok");
    assert_eq!(strip_ansi("\x1b]0;title\x07rest"), "rest");
    assert_eq!(strip_ansi("\x1b]8;;http://x\x1b\\link"), "link");
}

#[test]
fn centered_boxes_stay_on_screen() {
    assert_eq!(centered(20, 10, 30, 30, 3), Rect::new(0, 0, 20, 10));
    assert_eq!(centered(20, 10, 6, 4, 0), Rect::new(7, 3, 6, 4));
}

// ----- background commands (spec §15) ---------------------------------------

fn background_app(src: &str) -> (Engine, TestHost) {
    let (e, mut h) = app_with(model_of(src));
    h.background = true;
    (e, h)
}

#[test]
fn background_command_streams_to_log_and_ends() {
    let (mut e, mut h) = background_app(APP);
    h.run_results.insert("./backup.sh".into(), Outcome { stdout: "step 1\nstep 2\n".into(), ..Default::default() });
    key(&mut e, &mut h, KeyCode::Enter);
    assert_eq!(h.runs.len(), 1);
    assert!(e.has_jobs() && e.busy());
    assert!(e.log_lines("LOG").is_empty(), "nothing before the command writes");
    let s = screen(&mut e);
    assert!(s.lines().any(|l| l.ends_with('|')), "spinner in the menu area: {s}");
    h.finish(h.last_job().unwrap());
    e.poll_jobs(&mut h);
    assert_eq!(e.log_lines("LOG"), ["step 1", "step 2"]);
    assert!(!e.has_jobs());
}

#[test]
fn ui_keeps_working_while_a_command_runs() {
    let (mut e, mut h) = background_app(APP);
    key(&mut e, &mut h, KeyCode::Enter);
    key(&mut e, &mut h, KeyCode::Down);
    key(&mut e, &mut h, KeyCode::Enter);
    assert_eq!(e.top().as_deref(), Some("SETTINGS"), "menus open while BACKUP runs");
    assert!(e.has_jobs());
}

#[test]
fn same_command_is_not_started_twice() {
    let (mut e, mut h) = background_app(APP);
    key(&mut e, &mut h, KeyCode::Enter);
    typ(&mut e, &mut h, "b");
    assert_eq!(h.runs.len(), 1);
    assert_eq!(e.log_lines("LOG"), ["BACKUP is already running"]);
}

#[test]
fn back_cancels_the_running_command_without_continuing_the_chain() {
    let src = APP.replace("- env TARGET=$TARGET\n- output LOG", "- env TARGET=$TARGET\n- output LOG\n- then set USED 100\n- on_error set USED 1");
    let (mut e, mut h) = background_app(&src);
    key(&mut e, &mut h, KeyCode::Enter);
    let job = h.last_job().unwrap();
    key(&mut e, &mut h, KeyCode::Esc);
    assert_eq!(h.cancelled, [job]);
    h.finish(job);
    e.poll_jobs(&mut h);
    assert_eq!(e.log_lines("LOG"), ["BACKUP cancelled"]);
    assert_eq!(e.var("USED"), Some(&Value::Number(40.0)), "neither then nor on_error ran");
    key(&mut e, &mut h, KeyCode::Esc);
    assert_eq!(h.cancelled, [job], "nothing left to cancel");
}

#[test]
fn back_closes_a_modal_before_cancelling_a_command_from_elsewhere() {
    let (mut e, mut h) = background_app(APP);
    key(&mut e, &mut h, KeyCode::Enter);
    key(&mut e, &mut h, KeyCode::Down);
    key(&mut e, &mut h, KeyCode::Enter);
    key(&mut e, &mut h, KeyCode::Esc);
    assert!(h.cancelled.is_empty());
    assert_eq!(e.top(), None, "SETTINGS closed");
    key(&mut e, &mut h, KeyCode::Esc);
    assert_eq!(h.cancelled.len(), 1);
}

#[test]
fn chain_and_into_continue_when_the_command_ends() {
    let (mut e, mut h) = background_app(APP);
    h.run_results.insert("whoami".into(), Outcome { stdout: "seppo\n".into(), ..Default::default() });
    e.run_command(&mut h, "WHOAMI", None);
    assert_eq!(e.var("OUT"), Some(&Value::Text(String::new())));
    h.finish(h.last_job().unwrap());
    e.poll_jobs(&mut h);
    assert_eq!(e.var("OUT"), Some(&Value::Text("seppo".into())));

    h.run_results.insert("./restore.sh".into(), Outcome { status: 2, ..Default::default() });
    e.run_command(&mut h, "RESTORE", None);
    key(&mut e, &mut h, KeyCode::Enter);
    h.finish(h.last_job().unwrap());
    e.poll_jobs(&mut h);
    assert_eq!(e.top().as_deref(), Some("DONE"), "on_error after the command ended");
}

#[test]
fn background_output_without_area_shows_at_the_end() {
    let (mut e, mut h) = background_app(APP);
    h.run_results.insert("./show.sh".into(), Outcome { stdout: "hello\n".into(), ..Default::default() });
    e.run_command(&mut h, "SHOW", None);
    assert_eq!(e.top(), None);
    h.finish(h.last_job().unwrap());
    e.poll_jobs(&mut h);
    assert_eq!(e.top().as_deref(), Some("output"));
    assert!(screen(&mut e).contains("hello"));
}

// ----- numbered areas (spec §4) ----------------------------------------------

fn alt(c: char) -> InputEvent {
    InputEvent::Key(KeyCode::Char(c), Mods { alt: true, ..Default::default() })
}

#[test]
fn focus_by_area_number_in_legend_order() {
    let (mut e, mut h) = app();
    assert_eq!(e.focused_target().as_deref(), Some("MAIN_MENU"));
    // STATUS is not focusable, so MAIN_MENU is 1 and LOG is 2.
    e.handle(&mut h, alt('2'));
    assert_eq!(e.focused_target().as_deref(), Some("LOG"));
    key(&mut e, &mut h, KeyCode::F(1));
    assert_eq!(e.focused_target().as_deref(), Some("MAIN_MENU"));
    e.handle(&mut h, alt('9'));
    assert_eq!(e.focused_target().as_deref(), Some("MAIN_MENU"), "no area 9");
}

#[test]
fn focus_action_in_keys_and_menus() {
    let src = APP.replace("keys MAIN\n- BACKUP b", "keys MAIN\n- BACKUP b\n- focus 2 g\n\nmenu JUMP\n- LOGS focus 2");
    let (mut e, mut h) = app_with(model_of(&src));
    typ(&mut e, &mut h, "g");
    assert_eq!(e.focused_target().as_deref(), Some("LOG"));
    assert!(e.model.keymap.any_binding_for("focus 1").is_some(), "other numbers keep their defaults");
    assert!(e.model.keymap.any_binding_for("focus 2").is_some_and(|b| b.to_string() == "g"));
    let bad = APP.replace("keys MAIN\n- BACKUP b", "keys MAIN\n- focus 10 g");
    let blocks: Vec<_> = parse("app.termplate", &bad).unwrap().blocks().cloned().collect();
    let err = Model::build(blocks, &mut TestHost::new()).unwrap_err();
    assert!(err.to_string().contains("area number 1-9"), "{err}");
}

#[test]
fn area_numbers_in_borders() {
    let src = APP.replace("log LOG history 50", "log LOG history 50\n\npanels\n- LOG border single\n\nsettings\n- area_numbers on");
    let (mut e, _) = app_with(model_of(&src));
    let s = screen(&mut e);
    assert!(s.lines().any(|l| l.starts_with("┌2")), "{s}");
}

// ----- path completion (spec §8) ---------------------------------------------

fn path_input() -> (Engine, TestHost) {
    let (mut e, mut h) = app();
    h.paths = vec!["/mnt/backup/".into(), "/mnt/backups/".into(), "/mnt/data/".into(), "/srv/".into()];
    e.perform_named(&mut h, "open TARGET_DIR");
    assert_eq!(e.top().as_deref(), Some("TARGET_DIR"));
    (e, h)
}

#[test]
fn tab_completes_the_common_prefix() {
    let (mut e, mut h) = path_input();
    for _ in 0..12 {
        key(&mut e, &mut h, KeyCode::Backspace);
    }
    typ(&mut e, &mut h, "/s");
    key(&mut e, &mut h, KeyCode::Tab);
    assert!(screen(&mut e).contains("/srv/"), "single match: {}", screen(&mut e));
    assert_eq!(e.top().as_deref(), Some("TARGET_DIR"), "tab is not next_area in an input");
    for _ in 0..5 {
        key(&mut e, &mut h, KeyCode::Backspace);
    }
    typ(&mut e, &mut h, "/m");
    key(&mut e, &mut h, KeyCode::Tab);
    assert!(screen(&mut e).contains("/mnt/"), "{}", screen(&mut e));
}

#[test]
fn second_tab_lists_matches_and_choice_fills_the_input() {
    let (mut e, mut h) = path_input();
    // the input starts with /mnt/backup: matches /mnt/backup/ and /mnt/backups/
    key(&mut e, &mut h, KeyCode::Tab);
    assert_eq!(e.top().as_deref(), Some("TARGET_DIR"), "common prefix was already typed");
    key(&mut e, &mut h, KeyCode::Tab);
    assert_eq!(e.top().as_deref(), Some(COMPLETE_MENU));
    assert!(screen(&mut e).contains("/mnt/backups/"));
    key(&mut e, &mut h, KeyCode::Down);
    key(&mut e, &mut h, KeyCode::Enter);
    assert_eq!(e.top().as_deref(), Some("TARGET_DIR"));
    assert!(screen(&mut e).contains("/mnt/backups/"));
}

#[test]
fn back_closes_the_completion_list() {
    let (mut e, mut h) = path_input();
    key(&mut e, &mut h, KeyCode::Tab);
    key(&mut e, &mut h, KeyCode::Tab);
    key(&mut e, &mut h, KeyCode::Esc);
    assert_eq!(e.top().as_deref(), Some("TARGET_DIR"));
}

#[test]
fn long_completion_lists_are_cut() {
    let (mut e, mut h) = path_input();
    h.paths = (0..300).map(|i| format!("/mnt/backup{i:03}")).collect();
    key(&mut e, &mut h, KeyCode::Tab);
    key(&mut e, &mut h, KeyCode::Tab);
    assert_eq!(e.menu_state(COMPLETE_MENU).unwrap().total(), 201);
}

#[test]
fn load_warnings_go_to_the_first_log() {
    let src = APP.replace("panel HELP\nhhhhhhhhhh", "panel HELP\nhhhhhhhhhh 日");
    let (e, _) = app_with(model_of(&src));
    let lines = e.log_lines("LOG");
    assert!(lines.iter().any(|l| l.starts_with("warning: ") && l.contains("wide character")), "{lines:?}");
}

// ----- key commands on list rows, input `then` ---------------------------------

#[test]
fn key_bound_command_gets_the_selected_row_as_it() {
    let src = APP
        .replace("command SHOW\n- run ./show.sh", "command SHOW\n- confirm SHOW.CONFIRM\n- run ./show.sh\n\nkeys MENU\n- SHOW v")
        .replace("log LOG history 50", "log LOG history 50\n\ntext fi\nSHOW.CONFIRM Show {IT}?");
    let (mut e, mut h) = app_with(model_of(&src));
    e.set_var("FILES", Value::List(vec!["a.log".into(), "b.log".into()]));
    e.perform_named(&mut h, "open FILE_LIST");
    typ(&mut e, &mut h, "v");
    assert!(screen(&mut e).contains("Show b.log?"), "sorted desc, first row: {}", screen(&mut e));
    key(&mut e, &mut h, KeyCode::Enter);
    assert!(h.runs[0].env.contains(&("IT".into(), "b.log".into())), "{:?}", h.runs[0].env);
}

#[test]
fn input_then_runs_after_a_valid_value() {
    let src = APP.replace("- error TARGET_DIR.INVALID", "- error TARGET_DIR.INVALID\n- then BACKUP");
    let (mut e, mut h) = app_with(model_of(&src));
    e.perform_named(&mut h, "open TARGET_DIR");
    key(&mut e, &mut h, KeyCode::Enter);
    assert!(h.runs.is_empty(), "invalid value: no then");
    for _ in 0..12 {
        key(&mut e, &mut h, KeyCode::Backspace);
    }
    typ(&mut e, &mut h, "/ok");
    key(&mut e, &mut h, KeyCode::Enter);
    assert_eq!(h.runs.len(), 1);
    assert_eq!(h.runs[0].env.iter().find(|(k, _)| k == "TARGET").map(|(_, v)| v.as_str()), Some("/ok"));
}

#[test]
fn kept_list_clears_its_filter_after_a_choice() {
    let src = APP.replace("menu PICK list\n- source $FILES\n- filter\n- multi", "menu PICK list\n- source $FILES\n- filter\n- keep\n- then set OUT $it");
    let (mut e, mut h) = app_with(model_of(&src));
    e.set_var("FILES", Value::List(vec!["alpha".into(), "beta".into()]));
    e.perform_named(&mut h, "open PICK");
    typ(&mut e, &mut h, "bet");
    key(&mut e, &mut h, KeyCode::Enter);
    assert_eq!(e.var("OUT"), Some(&Value::Text("beta".into())));
    assert_eq!(e.menu_state("PICK").unwrap().filter, "");
    assert_eq!(e.top().as_deref(), Some("PICK"));
}

#[test]
fn select_marks_and_moves_down() {
    let (mut e, mut h) = widget_with_files("PICK", &["a", "b", "c"]);
    key(&mut e, &mut h, KeyCode::Insert);
    key(&mut e, &mut h, KeyCode::Insert);
    key(&mut e, &mut h, KeyCode::Enter);
    assert_eq!(e.take_effects(), [Effect::Exit { code: 0, output: Some("a\nb".into()) }]);
}

// ----- inputs in layout areas, `clear`, `stop` --------------------------------

const CHAT: &str = r#"app CHAT
- termplate 0.1

vars
- PROMPT text
- LAST text

layout MAIN
llll
iiii

legend:
- l LOG
- i ASK_LINE size.height 1

log LOG history 50

input ASK_LINE
- var PROMPT
- then ASK
- clear

command ASK
- run ./ask.sh
- env PROMPT=$PROMPT
- output LOG

command SERVE
- run ./serve.sh
- output LOG

menu M
- STOP stop SERVE
"#;

#[test]
fn layout_input_edits_in_place_and_clears_after_then() {
    let (mut e, mut h) = background_app(CHAT);
    key(&mut e, &mut h, KeyCode::Tab);
    assert_eq!(e.focused_target().as_deref(), Some("ASK_LINE"));
    typ(&mut e, &mut h, "hello");
    let f = e.render();
    assert!(f.rows().iter().any(|r| r.starts_with("hello")), "{:?}", f.rows());
    assert_eq!(f.cursor, Some((5, f.height - 1)));
    key(&mut e, &mut h, KeyCode::Enter);
    assert_eq!(h.runs[0].env, [("PROMPT".to_string(), "hello".to_string())]);
    assert_eq!(e.var("PROMPT"), Some(&Value::Text(String::new())));
    assert!(!screen(&mut e).contains("hello"), "field cleared");
    typ(&mut e, &mut h, "again");
    assert!(screen(&mut e).contains("again"));
}

#[test]
fn set_var_updates_a_layout_input() {
    let (mut e, mut h) = background_app(CHAT);
    key(&mut e, &mut h, KeyCode::Tab);
    typ(&mut e, &mut h, "draft");
    e.set_var("PROMPT", Value::Text("from host".into()));
    assert!(screen(&mut e).contains("from host"));
}

#[test]
fn stop_cancels_a_running_command() {
    let (mut e, mut h) = background_app(CHAT);
    e.run_command(&mut h, "SERVE", None);
    let job = h.last_job().unwrap();
    e.perform_named(&mut h, "open M");
    key(&mut e, &mut h, KeyCode::Enter);
    assert_eq!(h.cancelled, [job]);
    h.finish(job);
    e.poll_jobs(&mut h);
    assert_eq!(e.log_lines("LOG"), ["SERVE cancelled"]);
}
