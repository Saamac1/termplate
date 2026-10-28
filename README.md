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

## Usage

```sh
cargo build --release
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

- [`examples/backup`](examples/backup): the spec's example application.
- [`examples/filemanager`](examples/filemanager): a two-panel file manager in the style of Midnight Commander.
- [`examples/ollamaplate`](examples/ollamaplate): start Ollama, choose a model and chat.

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
