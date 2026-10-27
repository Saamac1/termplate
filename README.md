# Termplate

A template-based description language for text user interfaces, implemented
in Rust for macOS and Linux. The spec is in [`termplate.md`](termplate.md);
open questions and assumptions are in [`CLARIFICATIONS.md`](CLARIFICATIONS.md).

## Layout

| Crate | Role (spec §16) |
|---|---|
| `termplate-core` | Parsing, includes and patches, model and load-time validation, layout solving, panel field detection, texts, key contexts, menu state, the runtime engine and its drawing model, the coprocess line protocol. No I/O. |
| `termplate-term` | Terminal drawing (diffing) and input with crossterm on `/dev/tty`, themes, `NO_COLOR`, terminal restore. |
| `termplate-shell` | Shell host (§3 expressions, `sh -c` command runs with timeouts, background commands with streamed output and cancellation, path completion), JSON protocol, XDG overrides, the event loop and the `termplate` CLI. |

## Usage

```sh
cargo build --release
termplate check examples/backup/app.termplate      # validate
termplate run   examples/backup/app.termplate      # application mode
choice=$(termplate menu examples/backup/app.termplate MAIN_MENU)  # widget mode
coproc UI { termplate serve examples/backup/app.termplate MAIN_MENU; }  # coprocess mode
```

Widget mode (`menu`, `input`, `panel`) and `check` accept a file without an
`app` block; `run` and `serve` need one. Exit status: 0 choice, 1 back,
2 quit, 3 error.

## Examples

- [`examples/backup`](examples/backup): the spec's example application.
- [`examples/filemanager`](examples/filemanager): a two-panel file manager in the style of Midnight Commander.
- [`examples/ollamaplate`](examples/ollamaplate): start Ollama, choose a model and chat.

## Tests

```sh
cargo test                     # unit and CLI tests
python3 scripts/pty_smoke.py   # end-to-end on a pseudo-terminal (after cargo build)
```
