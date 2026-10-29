# Termplate

A template-based description language for text user interfaces, implemented
in Rust for macOS and Linux. The spec is in [`termplate.md`](termplate.md);
open questions and assumptions are in [`CLARIFICATIONS.md`](CLARIFICATIONS.md).

## Layout

| Crate | Role (spec §16) |
|---|---|
| `termplate-core` | Parsing, includes and patches, model and load-time validation, layout solving, panel field detection, texts, key contexts, menu state, the runtime engine and its drawing model, the coprocess line protocol. No I/O. |
| `termplate-term` | Terminal drawing (diffing) and input with crossterm on `/dev/tty`, styles, `NO_COLOR`, terminal restore. |
| `termplate-shell` | Shell host (§3 expressions, `sh -c` or `- shell` command runs with timeouts, `run:` scripts from private temporary files with a load-time `-n` check, `- window` commands in tmux or a new terminal window, background commands with streamed output and cancellation, path completion), JSON protocol, XDG overrides, the event loop and the `termplate` CLI. |

## Install

Needs Rust 1.85 or newer (edition 2024); get it from [rustup.rs](https://rustup.rs).

```sh
git clone https://github.com/Saamac1/termplate.git && cd termplate
cargo install --path crates/termplate-shell   # installs `termplate` into ~/.cargo/bin
termplate --help
```

If the shell does not find `termplate`, add `~/.cargo/bin` to `PATH`. To
update, run the `cargo install` line again; to remove, `cargo uninstall
termplate-shell`.

## Hello, world

One dialog in one file. Save this as `hello.termplate`:

```
panel HELLO
mmmmmmmmmmmmmmmmmmmm

legend:
- m text HELLO.MESSAGE "Hello, world!" center
```

and run it:

```sh
termplate panel hello.termplate HELLO
```

A box with *Hello, world!* opens; Enter or Esc closes it. The drawing is
the dialog's content: the run of `m` is a field 20 columns wide, and the
legend makes it a text, `Hello, world!` unless a language file says
otherwise, centred. The border comes from the style, not the drawing. A
single dialog needs no `app` block (widget mode). The same dialog can live
inside a shell script, as `-` reads the file from stdin:

```sh
termplate panel - HELLO <<'EOF'
panel HELLO
mmmmmmmmmmmmmmmmmmmm

legend:
- m text HELLO.MESSAGE "Hello, world!" center
EOF
```

## First test: the file manager

Run from the repository folder:

```sh
termplate run examples/filemanager/app.termplate
```

Two directory panels open in your home directory. Move with the arrow
keys, open a directory with Enter, switch panels with Tab and quit with
F10 or Ctrl+Q. The other keys are in the
[file manager's README](examples/filemanager/README.md).

## Usage

```sh
termplate check examples/backup/app.termplate      # validate
termplate run   examples/backup/app.termplate      # application mode
choice=$(termplate menu examples/backup/app.termplate MAIN_MENU)  # widget mode
coproc UI { termplate serve examples/backup/app.termplate MAIN_MENU; }  # coprocess mode
```

Widget mode (`menu`, `input`, `panel`) and `check` accept a file without an
`app` block; `run` and `serve` need one. In such a widget file a menu item
needs no action: choosing it prints its key. Exit status: 0 choice, 1 back,
2 quit, 3 error.

The file name `-` reads the Termplate file from stdin, so a whole program
can be one shell script (`serve` takes `<(…)` instead, as its stdin is the
protocol). Includes and the working directory are then the current folder,
and `--name NAME` names the application for the user's overrides:

```sh
choice=$(termplate menu - MAIN_MENU <<'EOF'
menu MAIN_MENU
- BACKUP "Back up"
- QUIT "Quit"
EOF
) || exit
```

## Examples

Run each one from the repository folder. Ctrl+Q quits (in Sudoku, `q`).

```sh
termplate run examples/filemanager/app.termplate   # two-panel file manager (Midnight Commander style)
termplate run examples/wizard/app.termplate        # setup wizard: a tour of the language's features
termplate run examples/backup/app.termplate        # the spec's example application (Finnish texts)
termplate run examples/sudoku/sudoku.termplate     # Sudoku in one file
termplate run examples/ollamaplate/app.termplate   # Ollama: start the server, choose a model, chat
```

- [`examples/filemanager`](examples/filemanager): a two-panel file manager in the style of Midnight Commander; file operations in `fm.sh`.
- [`examples/wizard`](examples/wizard): a step-by-step installer whose pages show the language's features: forms with validation, password and choice fields, toggles, a grid, path completion, a list with columns and marking, a background install with a progress bar, two languages. Its README maps each feature to where it is used.
- [`examples/backup`](examples/backup): the spec's example application: a status panel, a main menu, settings and a log. Also try widget mode: `termplate menu examples/backup/app.termplate MAIN_MENU` prints the chosen key.
- [`examples/sudoku`](examples/sudoku): Sudoku in one file: a grid with a cursor, digit keys through one command (`KEY`), colours, embedded bash.
- [`examples/ollamaplate`](examples/ollamaplate): start Ollama, choose a model and chat. Needs `ollama`, `curl` and `python3`.

## Tests

```sh
cargo test                     # unit, CLI and end-to-end tests (`termplate render`)
python3 scripts/pty_smoke.py   # terminal paths on a pseudo-terminal (after cargo build)
```

`termplate render` runs any mode without a terminal, driven by a steps
file, and prints the screens, events and result (spec §14.4). End-to-end
tests use it instead of a pseudo-terminal:

```sh
printf 'key j\nexpect Restore\nkey enter\nwait\n' |
  termplate render --size 80x24 --steps - menu examples/backup/app.termplate MAIN_MENU
# --- exit 0
# RESTORE
```
