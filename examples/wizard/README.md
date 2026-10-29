# Setup wizard

A step-by-step installer that tours Termplate's features. It collects a
profile, options, a theme, a folder and the parts to install, then
"installs" them: it creates the parts' folders and writes `wizard.conf`
into the target folder (by default `$TMPDIR/termplate-wizard-demo`).

```sh
termplate run examples/wizard/app.termplate
```

Press `w` (or choose *Setup wizard…*) to start. Enter goes on, Esc goes
back a page, Ctrl+Q quits. Texts are in English and Finnish (`LANG=fi_FI.UTF-8`).

| Page | What you do | Shows |
|---|---|---|
| Welcome | Next | a menu with a text, pages as modals on a stack |
| 1/5 Profile | name, email, password, edition | a form: Tab between fields, `validate` and error texts, a `password` field, a `choice` field (arrow keys) |
| 2/5 Options | verbose log, workers | `toggle` a bool, `adjust` a number in its range |
| 3/5 Theme | pick a cell | a grid with a cursor and colours (`ansi`), `then` with the cursor's row and column |
| 4/5 Folder | the target folder | a `path` input: Tab completes, a second Tab lists the matches |
| 5/5 Parts | mark parts with Insert | a list menu with `columns`, `filter` and `multi`; the marked rows arrive as `IT` |
| Ready | Install | a confirmation (`confirm`), then the installer |

## Where each feature is

| Feature | Spec | Where |
|---|---|---|
| Several files with `include` and wildcards | §2 | `app.termplate` |
| Start action | §2 | `- start INIT` (`app.termplate`), `INIT` (`ui/commands.termplate`) |
| Typed variables: text, number with a range, bool, choice, list | §3 | `vars` in `app.termplate` |
| Layout in the grid form | §4.1 | `layout MAIN grid` (`ui/layout.termplate`) |
| Drawn layout for a narrow terminal | §4 | `layout MAIN.narrow when width < 72` |
| Panel fields: `text`, `value`, named `[fields]`, multi-line field | §5 | `SUMMARY` |
| `refresh`, `bar`, `spinner`, `$( … )` values | §5 | `HEADER` |
| Grid with a cursor, `ansi`, `then` | §5.1 | `THEME_PICKER` (`ui/steps.termplate`) |
| Embedded scripts (`run:`, `- shell bash`) | §6.1 | `PICK_THEME`, `INSTALL` |
| `env`, `into`, `then`, `on_error`, `confirm`, `timeout`, `output` | §6 | `INSTALL`, `CHOOSE_PARTS` |
| Password on stdin, never in the environment | §6, §15 | `- secret PASSWORD stdin` in `INSTALL` |
| `stop` a running command | §6, §15 | *Stop installing* in `MAIN_MENU` |
| `interactive` (the pager gets the terminal) | §6 | `VIEW` |
| `window` (new tmux or terminal window) | §15.1 | `SHELL` |
| Menus: `open`, `close`, `back`, `quit`, `disabled when`, `context`, `final` | §7 | `MAIN_MENU`, `WELCOME`, `CONFIRM`, `DONE`, `FAILED` |
| List menu: `source`, `columns`, `row`, `filter`, `multi`, `then` | §7.1 | `PART_LIST` |
| Inputs: `text`, `password`, `choice`, `path`, `validate`, `error`, `then` | §8 | `ui/steps.termplate` |
| Form panel with `then` | §8.1 | `PROFILE` |
| Key contexts, one command for many keys (`KEY`) | §9 | `ui/keys.termplate`, `SET_WORKERS` (keys 1–8) |
| Texts: two languages, `{NAME}`, `{@KEY}`, `{^action}`, Termplate's own texts | §10 | `text/en.termplate`, `text/fi.termplate` |
| Palette, borders, terminal colours, `MENU.selected`, `LOG.stderr` | §11 | `colors`, `panels`, `logs` in `ui/keys.termplate` |
| Log with kept colours | §12 | `log LOG history 500 ansi keep` |

## Files

- `app.termplate`: the app block, the variables and the includes.
- `ui/layout.termplate`: the main screen.
- `ui/steps.termplate`: the wizard's pages and inputs.
- `ui/commands.termplate`: the commands, with their scripts embedded.
- `ui/keys.termplate`: keys and style.
- `text/en.termplate`, `text/fi.termplate`: the texts.

The installer writes only into the target folder; *Uninstall* removes
what it wrote.
