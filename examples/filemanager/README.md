# Termplate File Manager

Two directory panels in the style of Midnight Commander. The UI is
Termplate data (`app.termplate`, `ui/`, `text/`); the file operations are
in `fm.sh`.

```sh
termplate run examples/filemanager/app.termplate
```

| Key | Action |
|---|---|
| Tab, F1, F2 | switch panel (F1 left, F2 right) |
| Enter | open the directory (`../` goes up) |
| typing | search in the panel; Esc clears the search |
| Insert | mark a row and move down |
| F3 / F4 | view with `$PAGER` / edit with `$EDITOR` |
| F5 / F6 | copy / move the marked rows (or the row under the cursor) to the other panel |
| F7 | new directory |
| F8, Delete | delete |
| Ctrl+T | show hidden files |
| Ctrl+R | refresh |
| F10, Ctrl+Q | quit |

Both panels start in your home directory; change `LEFT_DIR` and
`RIGHT_DIR` with a user override (`~/.config/termplate/FILEMANAGER/overrides/vars.termplate`).

How it uses Termplate:

- The app's base key context is `MENU`. Each panel menu has its own
  context (`LEFT_PANEL`, `RIGHT_PANEL`) for its F-keys, and other keys
  fall through to the default menu keys.
- A command bound to a key gets the marked rows of the focused panel as
  `IT`; `fm.sh` reads them from the environment, never from the command
  line.
- `LEFT_OPEN` prints the next directory and stores it with `into
  LEFT_DIR`; the panel's `source` then lists the new directory.
