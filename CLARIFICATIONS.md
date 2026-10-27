# Termplate: problems and clarifications needed

This document covers the spec draft `termplate.md` (draft of 26.10.2026)
and its Rust implementation. RLML 0.3 background material
(`RLML-taustatiedot.zip`) has been read: *RLML-dataformaatit* (D),
*RLML-moottorin vastuut* (M) and the earlier *Ruutu* draft of this spec.
Everything Termplate inherits "as in RLML" is now implemented the RLML way.

The document has four parts:

- **A.** The questions that RLML settled, and what changed in the code.
- **B.** Places where Termplate goes beyond RLML or differs from it. These are decisions to confirm.
- **C.** Questions that are still open.
- **D.** Contradictions in the spec, portability problems, and the implementation status.

Section numbers (§) refer to `termplate.md`.

---

## A. Settled by RLML (now implemented that way)

| Topic | RLML rule | What changed |
|---|---|---|
| Block boundaries | A block ends at a blank line. `legend:` continues over blank lines until a new header. A drawing belongs to its header, even across a blank line, and may sit in a markdown code fence (D 2.2). | A row after a blank line is an error. A fenced drawing may contain blank lines. |
| Comments, free text | `>` followed by a space or the line end. Trailing `**…**` / `***…***` / `~~…~~`. `#` headings are skipped. Other free text is an error. Drawings have no comments (D 2.5, M 2.2). | Implemented. Language files allow only `>` comments. |
| Names | UPPER_CASE parts joined with dots; lower case is vocabulary (D 2.1). | Names are checked. A layout variant is a lower-case part (`MAIN.narrow`), like tile variants. |
| `include` | Look in the including file's folder, then the root. `*` matches one level, `**` any depth. Wildcards take data files and skip `_x`, `.x` and `README` files. Load in alphabetical path order, each file once (D 3, M 2.1). | Implemented. `.termplate`, `.rlml`, `.md` and `.rlml.md` are accepted. A wildcard matching nothing gives a warning. |
| `namespace` | One line at the start of the file. It prefixes the file's definitions; lookup goes A.B.N → A.N → N; `.N` refers to the root (D 4). | Implemented for definitions, text keys and colors. Errors list every name tried. |
| Replacement, `override` | A later definition with the same full name replaces the earlier one, with a warning when it comes from another file. `override` silences the warning, wins regardless of load order, and is an error when there is nothing to replace. Different kinds with the same name are an error (D 4.5). | Implemented. `check` prints the warnings. |
| `patch` / `new` / `drop` | `patch` before the header. Rows are matched by key. `new` adds and `drop` removes; an unknown key is an error that lists the existing keys. Patches apply after everything is loaded. Layouts and panels cannot be patched. Unnamed sections merge row by row (D 4.8). | Implemented. Key rows are matched by action (`open X` and `toggle X` count as whole names). |
| User overrides | Every block in `overrides/` is a patch. A new block needs `new KIND NAME`. `settings` keys need no `new` (D 3.2). | Implemented under `$XDG_CONFIG_HOME/termplate/APP/`. |
| Language files | `KEY text` lines with no header, e.g. `text/fi.rlml`. The text runs to the end of the line. `{@KEY}` must exist and must not form cycles, checked at load. `{^ACTION}` naming an unknown action is a load error; an unbound action shows as its name (D 10, M 7.3). | Implemented. A language file is recognised by its file name (a language code) **and** by its content (the first row is an UPPER_CASE key). |
| Sizes | `size WxH`, `size.width N`, `25%`, `*`. Conflicting sizes in the same column are an error. With one layout, `layout` is optional; with several and no `layout` field, it is an error (D 2.4, D 11.1). | Implemented. |
| Menus | `menu NAME[(params)] [list] [size WxH] [context K] [final]`. An item without an action runs the command of that name, else the engine action in lower case (`- QUIT` → `quit`). `TITLE` and `TEXT` are reserved. `final` menus ignore `back` and `close`, and loading checks that they can be left (D 11.2, M 7.5). | Implemented. `final` is now a menu attribute, not an item suffix (that was my earlier guess). |
| List menus | `row NAME: expr, …` gives the parameters of the text `<MENU>.ROW`. Rows are recomputed after each choice in a `keep` menu. With no rows and no `<MENU>.EMPTY` text, the menu is not opened (D 11.2, M 7.5). | Implemented. `columns` take their values from the same named row parameters. |
| `adjust` | `adjust FIELD a..b [pool NAME]`: left and right change the value; the pool pays for raises (D 11.2, M 7.5). | Implemented, including `pool`. |
| Key contexts | `keys CTX [modal\|passthrough]`. `ALWAYS` wins over every other context. A `modal` context (the default) blocks the contexts below it. A key bound in `ALWAYS` and elsewhere gives a warning. The default bindings are: up `up k`, down `down j`, confirm `enter space`, back `esc backspace`. An app row for the same action replaces the default (D 11.3, M 8). | Implemented. A menu's `context` replaces `MENU`. A modal panel uses `MENU`. |
| Key names | `ctrl`, `shift`, `alt`; `plus`, `minus`, `num0`–`num9`, `shift+tab`; mouse `click`, `rclick`, `mclick`, `wheel.up`, `wheel.down` with modifiers; `pad CTX [deadzone P%]` (D 11.3). | Implemented. My earlier names (`wheel_up`, `pageup` aliases, capitalised names) are gone. |
| Key rows with arguments | `- open PAUSE esc`, `- toggle DEBUG f12`. The same binding closes the menu it opened (D 11.3, M 8). | Implemented. `toggle` takes a bool variable or a panel. |
| Mouse in menus | Works without bindings: the pointer highlights a row, a click chooses it, the wheel scrolls (and adjusts), a click outside a menu is `back`, a click on a modal panel is `confirm`. `settings - mouse off` disables the mouse (M 8.1). | Implemented. |
| Logs | Messages wrap between words. Without `more`, an overflowing message is cut with `…`. With `more`, new messages show in pages with `<LOG>.MORE`; any key shows the next page and is not a command, and `back` skips the rest. `merge` joins repeats (D 11.4, M 7.4). | Implemented. Merged repeats show as `text (×3)`. |
| Panels | `panel NAME(params)`; `param` must be a header parameter. Arguments come from the layout legend (`- s STATS(x)`) or from `open P(…)`; a modal panel's arguments are computed once. Multi-line fields wrap; text that does not fit is cut with `…`. `text KEY NAME: expr` passes parameters to the text (D 11.5, M 7.7). | Implemented. Arguments follow RogueScript form: positional, then `name: value`. |
| Style | `colors` holds a palette (`- STONE 5a5a6e`). `panels` gives areas borders and colors. `logs` colours messages. A menu's look is looked up by its context name (D 9). | Implemented. My earlier role-based `colors` block is gone. |

---

## B. Termplate beyond RLML: decisions to confirm

1. **Embedded areas pass keys through.** RLML menus are always modal. In Termplate, a menu, log or input placed in a layout area pushes its context as *passthrough*, so the layout's own keys (`keys MAIN - BACKUP b` in §9) keep working while the menu has focus. Modal menus and panels block as in RLML. **OK?**

** S: This is ok **

2. **Base key context.** RLML names it with `game - keys CTX`. **Implemented as**: `app - keys CTX`, defaulting to the layout's name, which makes `keys MAIN` in §9 work. **OK?**

** S: This is ok **

3. **Cursor action names.** §9 uses `up`/`down`; RLML uses `cursor.north`/`cursor.south`. Both are accepted. Which should be canonical?

** S: RLML is more clear **

4. **Default language.** RLML has none and treats a missing `language` setting as an error. Termplate (§10) allows `app - language` plus `LANG`. **Implemented as**: settings → `LC_ALL`/`LC_MESSAGES`/`LANG` → app default → inline default → key.

** S: This is ok **

5. **Inline default texts and `text LANG` blocks** (§10) are Termplate additions. RLML language files have no header.

** S: This is ok **

6. **Colours.** RLML allows only palette names and hex values. Termplate also accepts the terminal's own colours as lower-case vocabulary (`red`, `dark_grey`, `default`), which respect the user's terminal theme. Role variants in style rows (`MENU.selected`, `MENU.disabled`, `LOG.stderr`, `.title`, …) and the border names (`single`, `double`, `rounded`, `heavy`, `ascii`, `none`) are my own design; RLML only shows `border single`. **Please confirm the syntax.**

** S: Variants are good, keep them **

7. **Variables are not namespaced.** RLML's namespaced kinds do not include map variables, and shell `$NAME` cannot contain dots. Commands, menus, panels, inputs, logs, layouts, text keys and colours are namespaced. A variable whose name equals a definition's name is a load error.

** S: This is ok **

8. **List rows without a `<MENU>.ROW` text.** RLML would show the key. **Implemented as**: the row values joined by two spaces, or `$it` when there is no `row`. This keeps one-file scripts translation-free.

** S: This is ok **

9. **`log` rows.** RLML `log` rows select message classes. Termplate logs show command output and `log` protocol messages, so a `log` block with rows is a load error.

** S: This is ok **

10. **Empty list in widget mode.** Without `<MENU>.EMPTY` the menu is not opened (RLML), so `termplate menu` exits with status 1 and prints nothing. **OK?**

** S: This is ok **

11. **ANSI in logs.** `log NAME ansi keep|strip` is a Termplate option (§12 says "by a setting"), default `strip`. `keep` currently shows the escapes as raw text.

** S: This is ok **

---

## C. Still open

### Big items

- **C1 Background execution** (§5, §15). Commands and `$( … )` still run synchronously, so the UI waits. Timeouts and the SIGTERM→SIGKILL escalation work, but `back` cannot cancel a running command. The concurrency model needs deciding first: parallel commands? skip a refresh while the previous one is still running? what does `back` cancel?

** S: Is it possible to solve tihs with opening background task in new terminal window?  **

**Answer:** It is possible (`open -a Terminal` / `osascript` on macOS, `$TERMINAL` or `x-terminal-emulator` on Linux), but I don't recommend it as the main fix:
- It does not work over SSH, in a Linux console, or inside tmux without special handling.
- Termplate cannot reliably get the exit code or output back from the other window, so `on_ok`/`on_error`, the log and `refresh` would all break.
- Which terminal app to open, and how, differs on every machine.

**Proposal:** keep commands in the same process but don't block the UI.
1. Each command runs as a child process. A reader thread streams its stdout and stderr into the log. The UI keeps drawing and shows a spinner in the area that started the command.
2. **Concurrency:** different commands may run in parallel. The same command never runs twice at once: a second start is ignored and gives a warning in the log. A periodic `refresh` is skipped while the previous one is still running.
3. **`back`** cancels the newest running command started from the current context (SIGTERM, then SIGKILL after the grace period). A second `back` works as normal.
4. **Interactive programs** (`$EDITOR`, `less`, `ssh`) get a new attribute `- terminal`. The UI is suspended and the program gets the whole terminal; the UI returns when it exits. Midnight Commander does the same (Ctrl-O), and it is what the file manager example needs.
5. A separate window can be added later as an opt-in `- window` attribute, if you still want it.

- **C2 Picture text vs. field markers** (§1 vs. §5). RLML admits the problem: "choose legend characters that the picture does not use" (D 11.5). The §5 example still puts the Finnish words `Levy:` and `Kuorma:` in a drawing. Should the spec say "use `text` fields or `[name___]`" and drop words from the example?

** S: spec should be "use `text` fields or `[name___]`" and move text to use text fields. 
 **

- **C3 Widget files without `app`** (§14.1 `termplate menu ui.termplate …` vs. §2 mandatory `app` and version). `app` is still required.

** S: what you mean? **

**Answer:** The two rules don't fit together:
- §2 says every root file starts with an `app NAME` block containing `- termplate VERSION`, and a missing version is a load error.
- §14.1 shows `termplate menu ui.termplate MAIN_MENU`. A file like `ui.termplate` that only holds a few menus for a shell script would then also need an `app` block, or it fails to load.

**Proposal:** in widget mode (`menu`, `input`, `panel`) the `app` block is optional.
- With no `app` block, the app name is the file name in upper case (`ui.termplate` → `UI`) and the newest supported version is assumed.
- `termplate check` gives a warning that the version is missing.
- User overrides still work under `$XDG_CONFIG_HOME/termplate/UI/`.
- `termplate run` still requires `app`.

- **C4 What reaches a command's environment** (§6). All non-secret variables and parameters plus the `env` entries and `IT` are passed. Should it be only `env`?

** S: Only env **

- **C5 Coprocess responsibilities** (§14.3). In `serve`, items and keys that refer to commands emit `select`/`key` and do not run; built-in actions run. Confirm.

** S: this is ok **

### Smaller questions

- **C6 `namespace` in text keys used by definitions.** Inside a namespace, the menu `MAIN` becomes `TOOLS.MAIN`, so its texts are `TOOLS.MAIN.TITLE`. A language file needs `namespace TOOLS` or full keys. OK?

** S: OK **

- **C7 Path completion** (§8) is not implemented. Tab is `next_area`. Which key completes, and how are multiple matches shown?

** S: we need to number areas **

**Answer / proposal** (please confirm this is what you meant):
- **Numbered areas:** areas are numbered 1–9 in the order they appear in the layout `legend:`. A new action `focus N` moves focus to area N. Default keys: `alt+1`…`alt+9`, plus `f1`…`f9` when the app does not bind them. Plain digits stay free for typing in inputs and for `number` fields. A setting can show the number in each area's border title.
- **Tab:**
  - In a `path` input, `tab` completes. Outside an input, `tab` stays `next_area`.
  - Input contexts are modal, so the binding does not clash.
- **Several matches:**
  - The first `tab` completes the common prefix.
  - The second `tab` opens a small list menu under the input with the matches (directories end in `/`). Choosing one fills it in, and `back` closes the list.
  - With more than 200 matches, the list says `…` and shows only the first 200.


- **C8 Forms** (§8, §17.1): still open. Today, `confirm` on a panel with an `input` field opens that input as a modal.

** S: this is OK **

- **C9 Borders** (§11, §17.2). Today: a modal box always has a border (style or `single`); a layout area gets one only from a `panels` row; a panel's own drawing may also draw one. That is close to "both; style draws a border only when the drawing has none", but the engine cannot tell whether a drawing contains a border.

** S: lets leave borders out from drawing. **

- **C10 Menu condition staleness.** RLML evaluates conditions when a menu opens; §7 adds "and when a variable changes". Filesystem-dependent conditions (`ok $(test -d …)`) can still go stale while a menu stays open. Should they also be re-evaluated on `refresh`, which the implementation does, or on a timer?

** S: refresh is ok **

- **C11 Expression details** (§3): numeric comparison when both sides are numbers; operators must be separate words; no parentheses; `$NAME` falls back to the process environment. RogueScript differs. Termplate's shell host keeps its own small language.

** S: OK **

- **C12 Exit codes** (§14.1): "muu virhe" is implemented as 3; a signal exits with 128+signal. `termplate panel` prints nothing.

** S: OK **

- **C13 JSON protocol** field names are invented: `{"cmd":"set","name":…,"value":…}` in, `{"event":"select","name":…,"value":…}` out. Errors go back as `error protocol MESSAGE` (not in the spec).

** S: OK **

- **C14 Password values** reach a command only when it declares `- secret VAR stdin|env`. §15's "by default on stdin" could also mean "without a declaration".

** S: OK **

- **C15 Unicode width.** RLML treats a wide character in a drawing as a load *warning* (M 2.5). Termplate counts characters and does not warn yet.

** S: should we write warning to log? **

**Answer:** Mostly no. Drawings are checked at load time, before any log area exists, so the warning belongs with the other load warnings (replacements, empty wildcards): `termplate check` prints them, and `run` prints them to stderr on start. In app mode it can *also* be added as the first message in the app's log, if the layout has one. That is cheap and helps the user notice it.

The engine should also measure width properly instead of counting characters, using the `unicode-width` crate. Then a wide character in a field value (a file name with emoji or CJK in the file manager) does not break the alignment of columns. The load warning would then be only for wide characters inside **drawings**, as in RLML.


- **C16 `pad`.** Bindings parse and are checked, but there is no gamepad input in a terminal. The Ruutu draft suggested dropping `pad`; Termplate §9 keeps it "for the Steam Deck". Is that still wanted?

** is it possible to use gilrs? **

**Answer:** Yes. `gilrs` supports macOS (IOKit) and Linux (evdev, so Steam Deck too). A small thread polls `gilrs` and sends button and stick events into the same event queue as crossterm's keys. The existing `pad CTX [deadzone P%]` bindings then just start receiving events.

Caveats:
- On Linux it needs `libudev` when building (`libudev-dev` package).
- On the Steam Deck, Steam Input may already turn the pad into keyboard and mouse input (desktop mode). The app then gets keys, which work anyway.
- Over SSH, the pad is on the wrong machine.

**Proposal:** an optional cargo feature `gamepad` (off by default, so normal builds need no udev) plus `settings - pad off`. It is low priority after C1 and C7.

- **C17 Termplate settings** in `settings.termplate`: `language`, `theme` (alias `style`), `refresh` (factor), `timeout`, `mouse`. Themes as files (§11) are not implemented. RLML also leaves the style selection open (D 13 "Tyylin rajaus").

** S: this is ok **

---

## D. Contradictions, portability and status

**Contradictions in the spec**

1. §1 "Texts as keys" vs. the drawing in §5 (C2).
2. §2 mandatory `app` vs. §14.1 widget files (C3).
3. §9 uses `up`/`down`; RLML uses `cursor.north`/`cursor.south` (B3).
4. §7 lists `final` with the menu features, but the §7 examples have no `final`. RLML shows it is a menu attribute, which is what the code now does.
5. §17 lists only two open questions; sections A–C list more.

** fix these **

**Portability (target: macOS and Linux)**

- `df -h --output=avail` (§5) is GNU-only.
- `/proc/loadavg` (§5) is Linux-only.
- `examples/backup` uses `df -h / | awk …` and `uptime` instead.

** S: we need better example file.

Implement Termplate File Manager like Midnight commander 

Another example - Ollamaplate , tui for starting ollama serve, choosing model from ollama list, show ollama ps and chat window ***

**Implementation status** (27.10.2026, after the answers above)

The spec (`termplate.md`) now follows the answers: B3, C1–C4, C7, C9, C15, C16 and D1–D5 are written in, and §17 lists what is still open.

Done in code:
- **C1 background commands.** Commands run as child processes and stream their output to the log while the UI keeps working. The same command is not started twice. `back` cancels a running command; a cancelled command does not continue its chain. Quitting stops running commands. Kept the existing `run … interactive` for programs that need the terminal, instead of a new `- terminal` attribute.
- **C3.** `check`, `menu`, `input` and `panel` accept a file without `app`.
- **C4.** Commands get only `env` entries, `IT` and declared secrets. `$( … )` in expressions still sees all variables, otherwise `ok $(test -d "$TARGET")` would break.
- **B3.** `{^…}` shows `cursor.north` etc. for unbound cursor actions; `up`/`down` still work as aliases.
- **C7.** `focus N` (keys `alt+N`, `fN`; key row `- focus 2 g`), numbered in legend order, counting only areas that can take focus. `settings - area_numbers on` shows the numbers. Tab completes in `path` inputs; a second Tab opens a filterable list of matches.
- **C9.** Load warning for a panel drawing that draws its own border. The backup example now uses `text` fields and style borders.
- **C15.** Text is measured in display columns (`unicode-width`); a wide character takes two cells. Wide characters in drawings give a load warning, and load warnings appear in the app's first log at start.

Still open or not started:
- Forms (C8), theme files (C17), rendering kept ANSI colours (B11), gamepad input with `gilrs` (C16).

Examples (28.10.2026): `examples/filemanager` (Midnight Commander style) and `examples/ollamaplate` (Ollama server, models, chat). Both are run on a pseudo-terminal here. Engine additions they needed, now in the spec:
- a command run by a key gets the focused list's marked rows (or the cursor row) as `IT`; confirm texts can use `{IT}`;
- `select` (Insert) marks and moves down; a `keep` list's search clears after a choice;
- inputs: `- then ACTION`, `- clear`, and an input in a layout area is edited in place;
- `stop COMMAND` cancels a running command.

Background evaluation (28.10.2026): panel fields, menu conditions and list sources that contain `$( … )` are evaluated on a worker thread; plain `$NAME` values stay immediate. Old values stay until new ones arrive, a pending target is not started again (it is evaluated once more if something changed meanwhile), and a menu waiting for its first result shows `TERMPLATE.LOADING` (`…`). Opening a menu or panel still evaluates at once, because widget mode needs to know whether a list is empty. Ollamaplate with raw `ollama ps` and no server now draws at once instead of freezing for ~17 s; it keeps its `curl` check so its menus fill in faster.

Tests: 352 in total, `cargo clippy --all-targets -D warnings` clean, `scripts/pty_smoke.py` (now also covers background commands, cancel and quit) passes on this Intel Mac. Linux was not run.
