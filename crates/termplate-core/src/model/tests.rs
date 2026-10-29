use super::*;
use crate::host::TestHost;
use crate::loader::{merge, MergeMode};
use crate::style::Color;
use crate::syntax::parse;

const APP: &str = "app T\n- termplate 0.1\n\n";

fn build(src: &str) -> Result<Model, LoadErrors> {
    let blocks: Vec<RawBlock> = parse("t.termplate", src).unwrap_or_else(|e| panic!("{e:?}")).blocks().cloned().collect();
    let (blocks, _) = merge(vec![], blocks, MergeMode::Base)?;
    Model::build(blocks, &mut TestHost::new())
}

fn ok(src: &str) -> Model {
    build(&format!("{APP}{src}")).unwrap_or_else(|e| panic!("{e}"))
}

fn err(src: &str) -> String {
    build(&format!("{APP}{src}")).unwrap_err().to_string()
}

fn b(s: &str) -> Binding {
    parse_binding(s).unwrap()
}

fn stack(names: &[&str]) -> Vec<crate::keys::StackEntry> {
    names.iter().map(|n| crate::keys::StackEntry::modal(n)).collect()
}

// ----- app -------------------------------------------------------------------

#[test]
fn app_block_fields() {
    let m = build("app BACKUPTOOL\n- termplate 0.1\n- language fi\n- keys MAIN_KEYS\n").unwrap();
    assert_eq!(m.app.name, "BACKUPTOOL");
    assert_eq!(m.app.version, "0.1");
    assert_eq!(m.app.language.as_deref(), Some("fi"));
    assert_eq!(m.texts.language.as_deref(), Some("fi"));
    assert_eq!(m.app.keys.as_deref(), Some("MAIN_KEYS"));
}

#[test]
fn version_is_required_and_checked() {
    assert!(build("app T\n- language fi\n").unwrap_err().to_string().contains("missing required field `- termplate VERSION`"));
    assert!(build("app T\n- termplate 9.0\n").unwrap_err().to_string().contains("unsupported termplate version 9.0"));
}

#[test]
fn app_start_action() {
    let m = build("app T\n- termplate 0.1\n- start INIT\n\nvars\n- A text\n\ncommand INIT\n- run ./init.sh\n- into A\n").unwrap();
    assert_eq!(m.app.start, Some(Action::Command("INIT".into())));
    let m = build("app T\n- termplate 0.1\n- start set A ready\n\nvars\n- A text\n").unwrap();
    assert!(matches!(m.app.start, Some(Action::Set { ref var, .. }) if var == "A"));
    assert!(build("app T\n- termplate 0.1\n- start NOPE\n").unwrap_err().to_string().contains("unknown command `NOPE`"));
    assert!(build("app T\n- termplate 0.1\n- start\n").unwrap_err().to_string().contains("`start ACTION`"));
}

#[test]
fn missing_or_duplicate_app() {
    assert!(build("menu M\n- QUIT\n").unwrap_err().to_string().contains("missing `app` block"));
    assert!(build("app A\n- termplate 0.1\n\napp B\n- termplate 0.1\n").unwrap_err().to_string().contains("only one"));
    assert!(build("app T\n- termplate 0.1\n- colour red\n").unwrap_err().to_string().contains("unknown field `colour`"));
}

#[test]
fn single_layout_is_the_default_and_several_need_a_choice() {
    let m = ok("layout ONLY\nm\n\nlegend:\n- m M\n\nmenu M\n- QUIT\n");
    assert_eq!(m.app.layout.as_deref(), Some("ONLY"));
    assert_eq!(m.app.keys.as_deref(), Some("ONLY"), "base key context defaults to the layout");
    let e = err("layout A\nm\n\nlegend:\n- m M\n\nlayout B\nm\n\nlegend:\n- m M\n\nmenu M\n- QUIT\n");
    assert!(e.contains("several layouts and no `- layout` in the app block: A, B"), "{e}");
}

#[test]
fn names_must_be_upper_case() {
    assert!(err("menu Main\n- QUIT\n").contains("invalid name `Main`"));
    assert!(err("menu M\n- quit_now\n").contains("UPPER_CASE text key"));
    assert!(err("vars\n- lower text\n").contains("invalid variable name"));
}

// ----- vars ------------------------------------------------------------------

#[test]
fn vars_types_and_defaults() {
    let m = ok("vars\n- TARGET text \"/mnt/backup\"\n- VERBOSE bool false\n- LEVEL number 3 range 1..9\n- FILES list\n- MODE choice fast safe default safe\n- SPEED choice slow fast\n");
    assert_eq!(m.var("TARGET").unwrap().default, Value::Text("/mnt/backup".into()));
    assert_eq!(m.var("VERBOSE").unwrap().default, Value::Bool(false));
    assert_eq!(m.var("LEVEL").unwrap().ty, VarType::Number { range: Some((1.0, 9.0)) });
    assert_eq!(m.var("FILES").unwrap().default, Value::List(vec![]));
    assert_eq!(m.var("MODE").unwrap().default, Value::Text("safe".into()));
    assert_eq!(m.var("SPEED").unwrap().default, Value::Text("slow".into()));
}

#[test]
fn vars_errors() {
    assert!(err("vars\n- A float 1\n").contains("unknown type `float`"));
    assert!(err("vars\n- A number 12 range 1..9\n").contains("outside 1..9"));
    assert!(err("vars\n- A bool maybe\n").contains("not a bool"));
    assert!(err("vars\n- A choice\n").contains("choice needs options"));
    assert!(err("vars\n- A choice x y default z\n").contains("not one of"));
}

// ----- layout ------------------------------------------------------------------

const LAYOUT: &str = "layout MAIN\nsssmm\nsssmm\nlllll\n\nlegend:\n- s STATUS size.width 30\n- m MAIN_MENU\n- l LOG size.height 6\n\npanel STATUS\nab\n\nlegend:\n- a value $X\n- b text STATUS.B\n\nmenu MAIN_MENU\n- QUIT\n\nlog LOG history 500 more\n";

#[test]
fn layout_with_legend() {
    let m = ok(LAYOUT);
    let l = &m.layouts["MAIN"][0];
    assert_eq!(l.areas[&'s'], AreaDef { target: "STATUS".into(), args: vec![] });
    assert_eq!(l.grid.columns.iter().map(|t| t.size).collect::<Vec<_>>(), [Some(Dim::Cells(30)), None]);
    assert_eq!(l.grid.rows.iter().map(|t| t.size).collect::<Vec<_>>(), [None, Some(Dim::Cells(6))]);
    assert_eq!(m.logs["LOG"], LogDef { name: "LOG".into(), history: 500, merge: false, more: true, keep_ansi: false });
}

#[test]
fn layout_size_forms() {
    let m = ok("layout L\nab\nac\n\nlegend:\n- a M size 20x10\n- b M size.width 25% size.height *\n- c M\n\nmenu M\n- QUIT\n");
    let l = &m.layouts["L"][0];
    assert_eq!(l.grid.columns.iter().map(|t| t.size).collect::<Vec<_>>(), [Some(Dim::Cells(20)), Some(Dim::Percent(25))]);
    assert_eq!(l.grid.rows.iter().map(|t| t.size).collect::<Vec<_>>(), [Some(Dim::Cells(5)), Some(Dim::Cells(5))], "`a` spans both rows: 10 is shared");
    let r = l.grid.solve(&m.area_sizes(l), 200, 50).rects;
    assert_eq!((r[&'a'].w, r[&'b'].w), (20, 180), "all columns sized: the extra goes to the last");
}

#[test]
fn layout_errors() {
    assert!(err(&format!("{LAYOUT}layout L\nab\n\nlegend:\n- a X\n")).contains("unknown panel or menu or log or input `X`"));
    assert!(err(&format!("{LAYOUT}layout L\nab\n\nlegend:\n- a LOG\n")).contains("area `b` has no legend"));
    assert!(err(&format!("{LAYOUT}layout L\na\n\nlegend:\n- a LOG\n- z LOG\n")).contains("`z` does not appear"));
    assert!(err(&format!("{LAYOUT}layout L\na\n\nlegend:\n- a LOG size.depth 3\n")).contains("unknown area option"));
    assert!(err(&format!("{LAYOUT}layout L\naba\n\nlegend:\n- a LOG\n- b LOG\n")).contains("not a rectangle"));
    assert!(err(&format!("{LAYOUT}layout L\nab\nac\n\nlegend:\n- a LOG\n- b LOG size.width 5\n- c LOG size.width 6\n")).contains("conflicting widths"));
    assert!(build("app T\n- termplate 0.1\n- layout NOPE\n").unwrap_err().to_string().contains("unknown layout `NOPE`"));
}

#[test]
fn layout_variants_and_choice() {
    let src = format!("{LAYOUT}\nlayout MAIN.narrow when width < 80\nm\nl\n\nlegend:\n- m MAIN_MENU\n- l LOG size.height 6\n");
    let m = ok(&src);
    assert_eq!(m.layouts["MAIN"].len(), 2);
    assert_eq!(m.app.layout.as_deref(), Some("MAIN"), "a variant is not a second layout");
    assert_eq!(m.choose_layout("MAIN", 120, 40).unwrap().name, "MAIN");
    assert_eq!(m.choose_layout("MAIN", 79, 40).unwrap().name, "MAIN.narrow");
}

const GRID_LAYOUT: &str = "layout MAIN grid\n- columns 30 *\n- rows * 6\n\nSTATUS MAIN_MENU\nLOG    LOG\n\n";

#[test]
fn grid_layout_is_the_same_as_the_drawing() {
    let rest = &LAYOUT[LAYOUT.find("panel STATUS").unwrap()..];
    let m = ok(&format!("{GRID_LAYOUT}{rest}"));
    let drawn = ok(LAYOUT);
    let (g, d) = (&m.layouts["MAIN"][0], &drawn.layouts["MAIN"][0]);
    let targets = |l: &LayoutDef| l.order.iter().map(|c| l.areas[c].clone()).collect::<Vec<_>>();
    assert_eq!(targets(g), targets(d), "same targets, sizes and numbering order");
    let solve = |m: &Model, l: &LayoutDef| {
        let r = l.grid.solve(&m.area_sizes(l), 100, 30).rects;
        l.order.iter().map(|c| r[c]).collect::<Vec<_>>()
    };
    assert_eq!(solve(&m, g), solve(&drawn, d));
}

#[test]
fn grid_layout_spans_variants_and_sizes() {
    let src = "layout L grid\nA B\nA C\n- columns 20% 30%\n\nlayout L.narrow grid when width < 40\nA\n\nmenu A\n- QUIT\n\nmenu B\n- QUIT\n\nmenu C\n- QUIT\n";
    let m = ok(src);
    let l = &m.layouts["L"][0];
    assert_eq!(l.order.iter().map(|c| l.areas[c].target.as_str()).collect::<Vec<_>>(), ["A", "B", "C"]);
    assert_eq!(l.grid.columns.iter().map(|t| t.size).collect::<Vec<_>>(), [Some(Dim::Percent(20)), Some(Dim::Percent(30))]);
    assert_eq!(m.choose_layout("L", 39, 20).unwrap().name, "L.narrow");
}

#[test]
fn grid_layout_errors() {
    let menus = "menu A\n- QUIT\n\nmenu B\n- QUIT\n";
    let e = |s: &str| err(&format!("{s}\n{menus}"));
    assert!(e("layout L grid\n- colums 1 *\n\nA B\n").contains("unknown grid layout option `colums` (columns, rows)"));
    assert!(e("layout L grid\n- columns 1\n\nA B\n").contains("`columns` gives 1 size(s) but the grid has 2 columns"));
    assert!(e("layout L grid\nA B\nA\n").contains("same number of areas"));
    assert!(e("layout L grid\nA B A\n").contains("area `A` is not a rectangle"));
    assert!(e("layout L grid\nA X\n").contains("unknown panel or menu or log or input `X`"));
    assert!(e("layout L grid\n- rows 3\n").contains("has no grid"));
    assert!(e("layout L\n- columns 1\n").contains("a drawn layout has no items"));
}

#[test]
fn base_layout_names() {
    assert_eq!(base_layout("MAIN"), "MAIN");
    assert_eq!(base_layout("MAIN.narrow"), "MAIN");
    assert_eq!(base_layout("UI.MAIN.narrow"), "UI.MAIN");
}

#[test]
fn panel_arguments_in_layout_legend() {
    let m = ok("layout L\ns\n\nlegend:\n- s STATS($TARGET, title: \"x\")\n\npanel STATS(who, title)\nww\n\nlegend:\n- w param who\n");
    let a = &m.layouts["L"][0].areas[&'s'];
    assert_eq!(a.target, "STATS");
    assert_eq!(a.args[0].0, None);
    assert_eq!(a.args[1].0.as_deref(), Some("title"));
    let e = err("layout L\ns\n\nlegend:\n- s STATS(a, b, c)\n\npanel STATS(who)\nww\n\nlegend:\n- w param who\n");
    assert!(e.contains("takes 1 argument(s), got 3"), "{e}");
    let e = err("layout L\ns\n\nlegend:\n- s STATS(nope: 1)\n\npanel STATS(who)\nww\n\nlegend:\n- w param who\n");
    assert!(e.contains("no parameter `nope`"), "{e}");
}

// ----- panels ------------------------------------------------------------------

#[test]
fn spec_panel_example() {
    let src = "panel STATUS refresh 2s\n+----------------------------+\n| TTTTTTTTTTTTTTTTTTTTTTTTTT |\n| Levy:   dddddddd           |\n| Kuorma: llllll             |\n| pppppppppppppppppppppppppp |\n+----------------------------+\n\nlegend:\n- T text STATUS.TITLE center\n- d value $(df -h --output=avail / | tail -1) right\n- l value $(cut -d' ' -f1 /proc/loadavg)\n- p bar $USED of 100\n";
    let m = ok(src);
    let p = &m.panels["STATUS"];
    assert_eq!(p.refresh, Some(Duration::from_secs(2)));
    assert_eq!((p.width, p.height), (30, 6));
    let f = |n: &str| p.fields.iter().find(|f| f.name == n).unwrap();
    assert_eq!(f("T").kind, FieldKind::Text { key: "STATUS.TITLE".into(), params: vec![] });
    assert_eq!(f("T").align, Align::Center);
    assert!(matches!(&f("d").kind, FieldKind::Value { expr, ansi: false } if expr.src == "$(df -h --output=avail / | tail -1)"));
    assert!(matches!(&f("p").kind, FieldKind::Bar { value, max } if value.src == "$USED" && max.src == "100"));
}

#[test]
fn panel_field_kinds() {
    let m = ok("vars\n- NAME text\n- BUSY bool\n\npanel P(who)\n[name__] s qq\n\nlegend:\n- name input NAME\n- s spinner when $BUSY\n- q param who\n");
    let p = &m.panels["P"];
    assert_eq!(p.fields[0].kind, FieldKind::Input { input: "NAME".into() });
    assert!(m.inputs.contains_key("NAME"), "implicit input created");
    assert_eq!(p.fields[2].kind, FieldKind::Param { name: "who".into() });
    assert!(err("panel P\nqq\n\nlegend:\n- q param who\n").contains("not a parameter of panel P"));
}

#[test]
fn panel_text_fields_with_parameters_and_default() {
    let m = ok("panel P\nTT UU\n\nlegend:\n- T text P.TITLE HOST: $(hostname), USER: $USER center\n- U text P.SUB \"Tila\"\n");
    let FieldKind::Text { key, params } = &m.panels["P"].fields[0].kind else { panic!() };
    assert_eq!(key, "P.TITLE");
    assert_eq!(params.iter().map(|(n, e)| format!("{n}={}", e.src)).collect::<Vec<_>>(), ["HOST=$(hostname)", "USER=$USER"]);
    assert_eq!(m.panels["P"].fields[0].align, Align::Center);
    assert_eq!(m.texts.defaults["P.SUB"], "Tila");
}

#[test]
fn panel_errors() {
    assert!(err("panel P refresh soon\nx\n\nlegend:\n- x value $A\n").contains("bad duration"));
    assert!(err("panel P\nx\n\nlegend:\n- x widget\n").contains("unknown field kind `widget`"));
    assert!(err("panel P\nx\n\nlegend:\n- x bar $A\n").contains("`bar VALUE of MAX`"));
    assert!(err("panel P\nx\n\nlegend:\n- x input NOPE\n").contains("unknown input or variable"));
    assert!(err("panel P\nx\n\nlegend:\n- y value $A\n").contains("`y` does not appear"));
    assert!(err("panel P\nx\n\nlegend:\n- x spinner $A\n").contains("spinner when"));
    assert!(err("panel P\n").contains("no drawing"));
}

#[test]
fn expressions_are_checked_by_host() {
    let blocks: Vec<RawBlock> = parse("t", &format!("{APP}panel P\nx\n\nlegend:\n- x value BAD\n")).unwrap().blocks().cloned().collect();
    let mut host = TestHost::new();
    host.reject.push("BAD".into());
    let e = Model::build(blocks, &mut host).unwrap_err().to_string();
    assert!(e.contains("bad expression `BAD`"), "{e}");
    assert!(e.contains("t:8:"), "{e}");
}

// ----- commands --------------------------------------------------------------------

const CMD: &str = "vars\n- TARGET text\n- VERBOSE bool\n- OUT text\n- PASS text\n\nlog LOG\n\npanel DONE(file)\nx\n\nlegend:\n- x param file\n\npanel FAILED\nx\n\nlegend:\n- x value $A\n\n";

#[test]
fn spec_command_example() {
    let m = ok(&format!(
        "{CMD}command BACKUP\n- confirm BACKUP.CONFIRM\n- run ./backup.sh\n- env TARGET=$TARGET VERBOSE=$VERBOSE\n- output LOG\n- then open DONE\n- on_error open FAILED\n- into OUT\n- secret PASS stdin\n"
    ));
    let c = &m.commands["BACKUP"];
    assert_eq!(c.confirm.as_deref(), Some("BACKUP.CONFIRM"));
    assert_eq!(c.run.as_deref(), Some("./backup.sh"));
    assert_eq!(c.env.iter().map(|(k, e)| format!("{k}={}", e.src)).collect::<Vec<_>>(), ["TARGET=$TARGET", "VERBOSE=$VERBOSE"]);
    assert_eq!(c.output.as_deref(), Some("LOG"));
    assert_eq!(c.then, Some(Action::Open { target: "DONE".into(), args: vec![] }));
    assert_eq!(c.secrets, [("PASS".to_string(), SecretMode::Stdin)]);
}

#[test]
fn command_run_interactive_and_chains() {
    let m = ok(&format!("{CMD}command VIEW\n- run less -- \"$TARGET\" interactive\n\ncommand BOTH\n- run true\n- then VIEW\n"));
    assert!(m.commands["VIEW"].interactive);
    assert_eq!(m.commands["VIEW"].run.as_deref(), Some("less -- \"$TARGET\""));
    assert_eq!(m.commands["BOTH"].then, Some(Action::Command("VIEW".into())));
}

#[test]
fn open_with_rlml_arguments() {
    let m = ok(&format!("{CMD}command C\n- run true\n- then open DONE(file: $TARGET)\n"));
    let Some(Action::Open { target, args }) = &m.commands["C"].then else { panic!() };
    assert_eq!(target, "DONE");
    assert_eq!(args[0].0.as_deref(), Some("file"));
    assert_eq!(args[0].1.src, "$TARGET");
    let m = ok(&format!("{CMD}command C\n- run true\n- then open DONE($(date +%H:%M))\n"));
    let Some(Action::Open { args, .. }) = &m.commands["C"].then else { panic!() };
    assert_eq!(args[0], (None, Expr::new("$(date +%H:%M)", "t.termplate", 26)));
    assert!(err(&format!("{CMD}command C\n- run true\n- then open DONE(a, b)\n")).contains("takes 1 argument"));
}

#[test]
fn command_errors() {
    assert!(err(&format!("{CMD}command C\n- run x\n- output DONE\n")).contains("`DONE` is a panel, not a log"));
    assert!(err(&format!("{CMD}command C\n- run x\n- into NOPE\n")).contains("unknown variable `NOPE`"));
    assert!(err(&format!("{CMD}command C\n- run x\n- then open NOWHERE\n")).contains("unknown menu or panel or input or layout `NOWHERE` (tried NOWHERE)"));
    assert!(err(&format!("{CMD}command C\n- run x\n- then MISSING\n")).contains("unknown command `MISSING`"));
    assert!(err(&format!("{CMD}command C\n- run x\n- env lower=1\n")).contains("NAME=VALUE"));
    assert!(err(&format!("{CMD}command C\n- confirm X\n")).contains("needs `run`, `run:` or `then`"));
    assert!(err(&format!("{CMD}command C\n- run x\n- secret PASS file\n")).contains("stdin|env"));
    assert!(err(&format!("{CMD}command C\n- run x\n- retries 3\n")).contains("unknown field `retries`"));
    assert!(err(&format!("{CMD}command C\n- run x\n- then toggle TARGET\n")).contains("needs a bool variable or a panel"));
}

#[test]
fn embedded_scripts_and_interpreters() {
    let m = ok(&format!("{CMD}command B\n- env TARGET=$TARGET\n- output LOG\nrun:\n```bash\nset -e\n# copy\necho \"$TARGET\"\n```\n\ncommand S\n- shell zsh\n- run print hi\n\ncommand P\nrun:\n```\nexit 0\n```\n"));
    let b = &m.commands["B"];
    assert!(b.script);
    assert_eq!(b.run.as_deref(), Some("set -e\n# copy\necho \"$TARGET\"\n"));
    assert_eq!(b.shell.as_deref(), Some("bash"), "the fence's language picks the interpreter");
    assert_eq!(m.commands["S"].shell.as_deref(), Some("zsh"));
    assert!(!m.commands["S"].script);
    assert_eq!(m.commands["P"].shell, None, "sh by default");
    let m = ok(&format!("{CMD}command B\n- shell /usr/local/bin/bash\nrun:\n```bash\ntrue\n```\n\ncommand Q\n- shell python3\nrun:\n```python\nprint(1)\n```\n"));
    assert_eq!(m.commands["B"].shell.as_deref(), Some("/usr/local/bin/bash"));
    assert_eq!(m.commands["Q"].shell.as_deref(), Some("python3"));
}

#[test]
fn embedded_script_errors() {
    assert!(err(&format!("{CMD}command C\n- run x\nrun:\n```\ny\n```\n")).contains("either `- run` or `run:`"));
    assert!(err(&format!("{CMD}command C\n- shell zsh\nrun:\n```bash\ny\n```\n")).contains("contradicts the script's language `bash`"));
    assert!(err(&format!("{CMD}command C\nrun:\n```python\nprint(1)\n```\n")).contains("name the interpreter with `- shell`"));
    assert!(err(&format!("{CMD}command C\n- shell zsh\n- then quit\n")).contains("`shell` needs"));
    // the host's syntax check names the line in the Termplate file
    let src = format!("{APP}{CMD}command C\nrun:\n```\necho a\nSYNTAX ERROR\n```\n");
    let line = src.lines().position(|l| l == "SYNTAX ERROR").unwrap() + 1;
    let e = build(&src).unwrap_err().to_string();
    assert!(e.contains(&format!("t.termplate:{line}: command `C`: syntax error")), "{e}");
}

#[test]
fn missing_interpreter_is_a_load_error() {
    let blocks: Vec<RawBlock> =
        parse("t.termplate", &format!("{APP}command C\n- shell fish\n- run echo\n")).unwrap().blocks().cloned().collect();
    let (blocks, _) = merge(vec![], blocks, MergeMode::Base).unwrap();
    let mut host = TestHost::new();
    host.reject.push("fish".into());
    assert!(Model::build(blocks, &mut host).unwrap_err().to_string().contains("interpreter `fish` not found"));
}

const GRID_BASE: &str = "vars\n- GRID text\n- ROW number\n- COL number 9\n- NAME text\n\ncommand PICK\n- run ./pick.sh\n\n";

#[test]
fn grid_cells_come_from_the_drawing() {
    let m = ok(&format!("{GRID_BASE}panel BOARD\na a a │ a\na a a │ a\n\nlegend:\n- a grid $GRID ansi cursor ROW COL then PICK\n"));
    let p = &m.panels["BOARD"];
    let FieldKind::Grid { expr, ansi, cells, cursor, then } = &p.fields[0].kind else { panic!() };
    assert_eq!(expr.src, "$GRID");
    assert!(ansi);
    assert_eq!(cells.len(), 2, "stacked runs are separate rows");
    assert_eq!(cells[0].iter().map(|r| r.x).collect::<Vec<_>>(), [0, 2, 4, 8]);
    assert_eq!(cells[1][3], Rect::new(8, 1, 1, 1));
    assert_eq!(cursor, &Some(("ROW".into(), "COL".into())));
    assert_eq!(then, &Some(Action::Command("PICK".into())));
    assert!(p.cursor_grid().is_some());
    assert_eq!(p.fields[0].align, Align::Center);
    let m = ok(&format!("{GRID_BASE}panel SHOW\nbb bb\n\nlegend:\n- b grid $GRID\n"));
    assert!(m.panels["SHOW"].cursor_grid().is_none(), "no cursor: display only");
}

#[test]
fn grid_errors() {
    let e = |legend: &str| err(&format!("{GRID_BASE}panel P\na a\na a\n\nlegend:\n{legend}\n"));
    assert!(e("- a grid $GRID cursor NAME COL").contains("needs number variables, `NAME`"));
    assert!(e("- a grid $GRID cursor ROW").contains("`cursor ROW COL`"));
    assert!(e("- a grid $GRID then PICK").contains("`then` needs a `cursor`"));
    assert!(e("- a grid").contains("`grid VALUE"));
    let ragged = err(&format!("{GRID_BASE}panel P\na a a\na a\n\nlegend:\n- a grid $GRID\n"));
    assert!(ragged.contains("same number of cells"), "{ragged}");
    let two = err(&format!("{GRID_BASE}panel P\na b\n\nlegend:\n- a grid $GRID cursor ROW COL\n- b grid $GRID cursor ROW COL\n"));
    assert!(two.contains("more than one grid with a cursor"), "{two}");
    let form = err(&format!("{GRID_BASE}panel P\na [name___]\n\nlegend:\n- a grid $GRID cursor ROW COL\n- name input NAME\n"));
    assert!(form.contains("both input fields and a grid"), "{form}");
}

#[test]
fn command_timeout() {
    let m = ok(&format!("{CMD}command C\n- run sleep 9\n- timeout 1m\n\ncommand D\n- run true\n"));
    assert_eq!(m.commands["C"].timeout, Some(Duration::from_secs(60)));
    assert_eq!(m.commands["D"].timeout, None);
    assert!(err(&format!("{CMD}command C\n- run x\n- timeout soon\n")).contains("bad duration `soon`"));
    assert!(err(&format!("{CMD}command C\n- run x interactive\n- timeout 5s\n")).contains("no timeout"));
    assert!(err(&format!("{CMD}command C\n- run x\n- window\n- timeout 5s\n")).contains("`timeout`"));
}

#[test]
fn window_commands() {
    let m = ok(&format!("{CMD}command A\n- run htop\n- window popup\n\ncommand B\n- run top\n- window\n- secret PASS env\n"));
    assert_eq!(m.commands["A"].window, Some(Window::Popup));
    assert_eq!(m.commands["B"].window, Some(Window::Tab));
    assert!(err(&format!("{CMD}command C\n- run x\n- window tabs\n")).contains("`window [tab|split|popup]`"));
    for bad in ["- output LOG", "- into OUT", "- run x interactive"] {
        let e = err(&format!("{CMD}command C\n- run x\n- window\n{bad}\n"));
        assert!(e.contains("cannot be combined with"), "{bad}: {e}");
    }
    assert!(err(&format!("{CMD}command C\n- run x\n- window split\n- secret PASS stdin\n")).contains("secret on stdin"));
}

#[test]
fn single_file_app_with_several_languages() {
    let mut m = ok("menu M\n- QUIT \"Quit\"\n\ntext fi\nM.QUIT Lopeta\n\ntext en\nM.QUIT Exit\n");
    m.texts.language = Some("fi".into());
    assert_eq!(m.texts.lookup("M.QUIT"), Some("Lopeta"));
    m.texts.language = Some("en".into());
    assert_eq!(m.texts.lookup("M.QUIT"), Some("Exit"));
    m.texts.language = Some("sv".into());
    assert_eq!(m.texts.lookup("M.QUIT"), Some("Quit"));
}

// ----- menus ----------------------------------------------------------------------

const MENU_BASE: &str = "vars\n- TARGET text\n- VERBOSE bool\n- LEVEL number 3 range 1..9\n- POINTS number 5\n\ncommand BACKUP\n- run ./backup.sh\n\ncommand RESTORE\n- run ./restore.sh\n\n";

#[test]
fn spec_menu_example() {
    let m = ok(&format!(
        "{MENU_BASE}menu MAIN_MENU\n- BACKUP\n- RESTORE disabled when not ok $(test -d $TARGET)\n- SETTINGS open SETTINGS\n- QUIT quit\n\nmenu SETTINGS context POPUP\n- VERBOSE toggle VERBOSE\n- LEVEL adjust LEVEL 1..9\n- BACK back\n"
    ));
    let MenuKind::Static(items) = &m.menus["MAIN_MENU"].kind else { panic!() };
    assert_eq!(items[0].action, Action::Command("BACKUP".into()));
    assert_eq!(items[1].disabled_when.as_ref().unwrap().src, "not ok $(test -d $TARGET)");
    assert_eq!(items[2].action, Action::Open { target: "SETTINGS".into(), args: vec![] });
    assert_eq!(items[3].action, Action::Quit);
    assert_eq!(m.menus["SETTINGS"].context(), "POPUP");
    assert_eq!(m.menus["MAIN_MENU"].context(), "MENU");
    let MenuKind::Static(items) = &m.menus["SETTINGS"].kind else { panic!() };
    assert_eq!(items[0].action, Action::Toggle("VERBOSE".into()));
    assert_eq!(items[1].action, Action::Adjust { var: "LEVEL".into(), lo: 1.0, hi: 9.0, pool: None });
    assert_eq!(items[2].action, Action::Back);
}

#[test]
fn item_without_action_falls_back_to_lower_case_builtin() {
    // D 11.2: `- RESUME` → command RESUME, else action `resume`
    let m = ok(&format!("{MENU_BASE}menu M\n- QUIT\n- BACK\n- BACKUP\n"));
    let MenuKind::Static(items) = &m.menus["M"].kind else { panic!() };
    assert_eq!(items[0].action, Action::Quit);
    assert_eq!(items[1].action, Action::Back);
    assert_eq!(items[2].action, Action::Command("BACKUP".into()));
    assert!(err(&format!("{MENU_BASE}menu M\n- NOPE\n")).contains("no command `NOPE` or action `nope`"));
}

#[test]
fn menu_header_options() {
    let m = ok(&format!("{MENU_BASE}menu M size 24x* context POPUP final\n- QUIT\n- BACKUP\n"));
    let d = &m.menus["M"];
    assert_eq!(d.size, (Some(Dim::Cells(24)), None));
    assert!(d.final_);
    assert!(err(&format!("{MENU_BASE}menu M final\n- BACKUP\n- QUIT disabled when $VERBOSE\n")).contains("final menu `M` needs an item"));
    assert!(err(&format!("{MENU_BASE}menu M wide\n- BACKUP\n")).contains("unknown menu option `wide`"));
}

#[test]
fn menu_item_tail_options_and_reserved_names() {
    let m = ok(&format!("{MENU_BASE}menu M\n- BACKUP \"Varmuuskopioi\" hide when $VERBOSE disabled when not $TARGET\n- RESTORE disabled when $A hide when $B\n"));
    let MenuKind::Static(items) = &m.menus["M"].kind else { panic!() };
    assert_eq!(items[0].hide_when.as_ref().unwrap().src, "$VERBOSE");
    assert_eq!(items[0].disabled_when.as_ref().unwrap().src, "not $TARGET");
    assert_eq!(m.texts.defaults["M.BACKUP"], "Varmuuskopioi");
    assert_eq!(items[1].hide_when.as_ref().unwrap().src, "$B");
    assert!(err(&format!("{MENU_BASE}menu M\n- TITLE quit\n")).contains("reserved"));
}

#[test]
fn adjust_with_pool() {
    let m = ok(&format!("{MENU_BASE}menu M\n- LEVEL adjust LEVEL 1..9 pool POINTS\n"));
    let MenuKind::Static(items) = &m.menus["M"].kind else { panic!() };
    assert_eq!(items[0].action, Action::Adjust { var: "LEVEL".into(), lo: 1.0, hi: 9.0, pool: Some("POINTS".into()) });
    assert!(err(&format!("{MENU_BASE}menu M\n- X adjust LEVEL 1..9 pool TARGET\n")).contains("`TARGET` is not one"));
    assert!(err(&format!("{MENU_BASE}menu M\n- X adjust LEVEL 9..1\n")).contains("bad range"));
}

#[test]
fn menu_errors() {
    assert!(err(&format!("{MENU_BASE}menu M\n- X set NOPE 1\n")).contains("unknown variable `NOPE`"));
    assert!(err(&format!("{MENU_BASE}menu M\n- X adjust TARGET 1..2\n")).contains("needs number variables"));
    assert!(err(&format!("{MENU_BASE}menu M\n- BACKUP\n- BACKUP\n")).contains("duplicate menu item"));
    assert!(err(&format!("{MENU_BASE}menu M\n- X quit now\n")).contains("takes 0 argument"));
}

#[test]
fn spec_list_menu_example_with_rlml_row() {
    let m = ok("menu LOGS list\n- source $(ls /var/log/*.log)\n- row FILE: $it\n- sort $it desc\n- filter\n- then run \"less -- \\\"$IT\\\"\"\n- keep\n");
    let MenuKind::List(l) = &m.menus["LOGS"].kind else { panic!() };
    assert_eq!(l.source.src, "$(ls /var/log/*.log)");
    assert_eq!(l.row.len(), 1);
    assert_eq!((l.row[0].0.as_str(), l.row[0].1.src.as_str()), ("FILE", "$it"));
    assert_eq!(l.sort.as_ref().map(|(e, d)| (e.src.as_str(), *d)), Some(("$it", true)));
    assert!(l.filter && l.keep && !l.multi);
    assert_eq!(l.then, Some(Action::Run { cmdline: "less -- \"$IT\"".into(), interactive: false }));
}

#[test]
fn list_menu_with_parameters_and_columns() {
    let m = ok("menu L(dir) list\n- source $(ls $dir)\n- columns NAME:* SIZE:8 right\n- row NAME: $it, SIZE: $(du -sh $it | cut -f1)\n- multi\n");
    let d = &m.menus["L"];
    assert_eq!(d.params[0].name, "dir");
    let MenuKind::List(l) = &d.kind else { panic!() };
    assert_eq!(l.columns[1], Column { name: "SIZE".into(), width: Some(8), align: Align::Right });
    assert_eq!(l.row[1].1.src, "$(du -sh $it | cut -f1)");
    assert!(l.multi);
}

#[test]
fn list_menu_errors() {
    assert!(err("menu L list\n- row X: $it\n").contains("needs `source`"));
    assert!(err("menu L list\n- source x\n- filter yes\n").contains("takes no value"));
    assert!(err("menu L list\n- source x\n- columns NAME\n").contains("NAME:WIDTH"));
    assert!(err("menu L list\n- source x\n- columns A:*\n- row B: $it\n").contains("unknown column `B`"));
    assert!(err("menu L list\n- source x\n- row $it\n").contains("NAME: expression"));
    assert!(err("menu L list\n- source x\n- page 3\n").contains("unknown field `page`"));
}

// ----- inputs ---------------------------------------------------------------------

#[test]
fn spec_input_example() {
    let m = ok("vars\n- TARGET text\n\ninput TARGET_DIR\n- var TARGET\n- kind path\n- validate ok $(test -d \"$VALUE\")\n- error TARGET_DIR.INVALID\n");
    let i = &m.inputs["TARGET_DIR"];
    assert_eq!(i.var, "TARGET");
    assert_eq!(i.kind, InputKind::Path);
    assert_eq!(i.validate.as_ref().unwrap().src, "ok $(test -d \"$VALUE\")");
}

#[test]
fn input_errors() {
    assert!(err("input I\n- kind text\n").contains("needs `var`"));
    assert!(err("vars\n- A text\n\ninput I\n- var A\n- kind secret\n").contains("unknown input kind"));
    assert!(err("vars\n- A text\n\ninput I\n- var A\n- kind choice\n").contains("needs a choice variable"));
    assert!(err("vars\n- A text\n\ninput I\n- var A\n- kind number\n").contains("needs a number variable"));
}

// ----- keys -----------------------------------------------------------------------

#[test]
fn spec_keys_example() {
    let m = ok(&format!(
        "{MENU_BASE}layout MAIN\nm\n\nlegend:\n- m M\n\nmenu M\n- BACKUP\n\nkeys ALWAYS\n- quit ctrl+q\n\nkeys MENU\n- confirm enter space\n- back esc\n- up k up\n- down j down\n\nkeys MAIN\n- BACKUP b\n"
    ));
    let km = &m.keymap;
    assert_eq!(km.lookup(&stack(&["MAIN", "MENU"]), &b("space")), Some("confirm"));
    assert_eq!(km.lookup(&stack(&["MAIN", "MENU"]), &b("k")), Some("up"));
    assert_eq!(km.lookup(&stack(&["MAIN"]), &b("b")), Some("BACKUP"));
    assert_eq!(km.lookup(&stack(&["MENU"]), &b("pageup")), Some("page_up"), "untouched defaults survive");
    // the app's MENU back row replaced the default `esc backspace`
    assert_eq!(km.lookup(&stack(&["MENU"]), &b("backspace")), None);
}

#[test]
fn keys_with_open_toggle_modes_and_aliases() {
    let m = ok("vars\n- V bool\n\npanel DEBUG\nx\n\nlegend:\n- x value 1\n\nmenu PAUSE\n- QUIT\n\nmenu S context POPUP\n- BACK\n\nkeys ALWAYS\n- open PAUSE esc\n- toggle DEBUG f12\n- toggle V f2\n\nkeys POPUP passthrough\n- close backspace\n- cursor.north i\n");
    let km = &m.keymap;
    assert_eq!(km.lookup(&stack(&["MENU"]), &b("esc")), Some("open PAUSE"));
    assert_eq!(km.lookup(&stack(&["MENU"]), &b("f12")), Some("toggle DEBUG"));
    assert_eq!(km.lookup(&stack(&["MENU"]), &b("f2")), Some("toggle V"));
    assert_eq!(km.mode("POPUP"), Mode::Passthrough);
    assert_eq!(km.lookup(&stack(&["MENU", "POPUP"]), &b("i")), Some("up"));
    assert_eq!(km.lookup(&stack(&["MENU", "POPUP"]), &b("j")), Some("down"), "passthrough to MENU");
    assert!(m.warnings.iter().any(|w| w.message.contains("`Esc` is bound in ALWAYS")), "{:?}", m.warnings);
}

#[test]
fn pad_blocks_are_not_supported() {
    let blocks = parse("t.termplate", "pad MENU\n- confirm south\n").unwrap_err();
    assert!(blocks[0].message.contains("gamepad `pad` blocks are not supported"), "{blocks:?}");
}

#[test]
fn keys_errors() {
    assert!(err("keys NOWHERE\n- quit q\n").contains("unknown key context `NOWHERE`"));
    assert!(err("keys MENU\n- jump j\n").contains("unknown action `jump`"));
    assert!(err("keys MENU\n- BACKUP b\n").contains("unknown command `BACKUP`"));
    assert!(err("keys MENU\n- up hyper+k\n").contains("unknown modifier"));
    assert!(err("keys MENU\n- up k\n- down k\n").contains("bound to both `up` and `down`"));
    assert!(err("keys MENU\n- up k\n- up j\n").contains("duplicate action"));
    assert!(err("keys MENU\n- up\n").contains("needs at least one key"));
    assert!(err("keys MENU sticky\n- up k\n").contains("unknown keys option"));
}

// ----- texts ------------------------------------------------------------------------

#[test]
fn text_blocks() {
    let m = ok("text fi\nMAIN_MENU.BACKUP Varmuuskopioi nyt\n- MAIN_MENU.QUIT \"Lopeta\"\n\ntext en\nMAIN_MENU.BACKUP Back up\n");
    assert_eq!(m.texts.langs["fi"]["MAIN_MENU.BACKUP"], "Varmuuskopioi nyt");
    assert_eq!(m.texts.langs["fi"]["MAIN_MENU.QUIT"], "Lopeta");
    assert_eq!(m.texts.langs["en"]["MAIN_MENU.BACKUP"], "Back up");
    assert!(err("text fi\nlower nope\n").contains("must be UPPER_CASE"));
}

#[test]
fn text_references_are_checked_at_load() {
    assert!(err("text fi\nA {@MISSING}\n").contains("unknown text key `{@MISSING}`"));
    assert!(err("text fi\nA a{@B}\nB b{@A}\n").contains("text reference cycle: A → B → A"));
    assert!(err("text fi\nA press {^jump}\n").contains("unknown action or command `{^jump}`"));
    let m = ok("command GO\n- run true\n\ntext fi\nA {@B} {^confirm} {^GO} {^cursor.north}\nB b\n");
    assert_eq!(m.texts.langs["fi"].len(), 2);
}

#[test]
fn settings_language_overrides_app_language() {
    let m = build("app T\n- termplate 0.1\n- language fi\n\nsettings\n- language en\n- timeout 30s\n- refresh 2\n- mouse off\n").unwrap();
    assert_eq!(m.texts.language.as_deref(), Some("en"));
    assert_eq!(m.texts.fallback_language.as_deref(), Some("fi"));
    assert_eq!(m.settings.timeout, Some(Duration::from_secs(30)));
    assert_eq!(m.settings.mouse, Some(false));
    assert!(err("settings\n- volume 11\n").contains("unknown setting"));
    assert!(err("settings\n- mouse maybe\n").contains("mouse on|off"));
}

// ----- style --------------------------------------------------------------------------

#[test]
fn colors_panels_and_logs_sections() {
    let m = ok("log LOG\n\nmenu M\n- QUIT\n\ncolors\n- STONE 5a5a6e\n\npanels\n- LOG border single fg STONE bg black\n- MENU.selected fg black bg cyan bold\n- M.disabled fg dark_grey\n\nlogs\n- LOG.stderr fg red\n");
    assert_eq!(m.style.palette["STONE"], Color::Rgb(0x5a, 0x5a, 0x6e));
    assert_eq!(m.style.rows["LOG"].border, Some(Border::Single));
    assert_eq!(m.style.rows["MENU.selected"].style.bg, Some(Color::Cyan));
    assert_eq!(m.style.rows["M.disabled"].style.fg, Some(Color::DarkGrey));
    assert_eq!(m.style.rows["LOG.stderr"].style.fg, Some(Color::Red));
    assert_eq!(m.area_border("LOG"), Some(Border::Single));
}

#[test]
fn style_errors() {
    assert!(err("colors\n- STONE #5a5a6e\n").contains("six lower-case hex digits"));
    assert!(err("colors\n- stone 5a5a6e\n").contains("must be UPPER_CASE"));
    assert!(err("panels\n- NOWHERE fg red\n").contains("unknown menu or panel or log or input `NOWHERE`"));
    assert!(err("log LOG\n\npanels\n- LOG fg mauve\n").contains("unknown color `mauve`"));
}

#[test]
fn log_options() {
    let m = ok("log L ansi keep merge\n");
    assert!(m.logs["L"].keep_ansi && m.logs["L"].merge);
    assert!(err("log L history lots\n").contains("bad history"));
    assert!(err("log L forever\n").contains("unknown log option"));
    assert!(err("log L\n- MESSAGE.COMBAT\n").contains("takes no rows"));
}

// ----- namespaces ---------------------------------------------------------------------

#[test]
fn namespaces_resolve_references() {
    let src = "namespace TOOLS\nmenu MAIN\n- RUN\n- OTHER open .ROOT_MENU\n\ncommand RUN\n- run true\n";
    let mut blocks: Vec<RawBlock> = parse("tools.termplate", src).unwrap().blocks().cloned().collect();
    blocks.extend(parse("app.termplate", &format!("{APP}menu ROOT_MENU\n- GO open TOOLS.MAIN\n\nmenu MAIN\n- QUIT\n")).unwrap().blocks().cloned());
    let (blocks, _) = merge(vec![], blocks, MergeMode::Base).unwrap();
    let m = Model::build(blocks, &mut TestHost::new()).unwrap_or_else(|e| panic!("{e}"));
    assert!(m.menus.contains_key("TOOLS.MAIN") && m.menus.contains_key("MAIN"));
    let MenuKind::Static(items) = &m.menus["TOOLS.MAIN"].kind else { panic!() };
    assert_eq!(items[0].action, Action::Command("TOOLS.RUN".into()));
    assert_eq!(items[1].action, Action::Open { target: "ROOT_MENU".into(), args: vec![] });
    let MenuKind::Static(items) = &m.menus["ROOT_MENU"].kind else { panic!() };
    assert_eq!(items[0].action, Action::Open { target: "TOOLS.MAIN".into(), args: vec![] });
}

#[test]
fn durations() {
    assert_eq!(parse_duration("2s"), Some(Duration::from_secs(2)));
    assert_eq!(parse_duration("500ms"), Some(Duration::from_millis(500)));
    assert_eq!(parse_duration("0500ms"), Some(Duration::from_millis(500)), "leading zeros (D 2.4)");
    assert_eq!(parse_duration("1m"), Some(Duration::from_secs(60)));
    assert_eq!(parse_duration("2"), None);
}

#[test]
fn many_errors_reported_together() {
    let e = build(&format!("{APP}vars\n- bad text\n\nmenu M\n- NOPE\n\nkeys NOWHERE\n- quit q\n")).unwrap_err();
    assert_eq!(e.0.len(), 3, "{e}");
}

#[test]
fn wide_characters_in_drawings_warn() {
    let m = ok("panel P\nx 日本\n\nlegend:\n- x value 1\n\npanel Q\n[name_]\n\nlegend:\n- name value 日本\n");
    let w: Vec<&LoadError> = m.warnings.iter().filter(|w| w.message.contains("wide character")).collect();
    assert_eq!(w.len(), 1, "values may be wide, drawings may not: {:?}", m.warnings);
    assert!(w[0].message.contains("`日` in the drawing of panel P"), "{}", w[0].message);
}

#[test]
fn borders_in_panel_drawings_warn() {
    let m = ok("panel P\n+----+\n|xxxx|\n+----+\n\nlegend:\n- x value 1\n\npanel Q\n----\nxxxx\n\nlegend:\n- x value 1\n");
    let w: Vec<&LoadError> = m.warnings.iter().filter(|w| w.message.contains("own border")).collect();
    assert_eq!(w.len(), 1, "{:?}", m.warnings);
    assert!(w[0].message.contains("panel P"));
}
