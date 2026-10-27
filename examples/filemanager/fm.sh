#!/bin/sh
# Helper for the Termplate file manager. Values arrive in the environment
# (spec §6): DIR, SRC, DST, NAME, HIDDEN, and IT for the selected row(s),
# one name per line. Directory names end in `/`, as `ls -p` prints them.

# Expands `~` and makes a directory absolute.
absdir() {
  case "$1" in
    "" | "~") d=$HOME ;;
    "~/"*) d=$HOME/${1#"~/"} ;;
    *) d=$1 ;;
  esac
  (cd -- "$d" 2>/dev/null && pwd -P) || printf '%s\n' "$HOME"
}

# A directory for showing: under HOME as `~/…`.
tilde() {
  h=$(cd -- "$HOME" 2>/dev/null && pwd -P)
  case "$1" in
    "$h") echo "~" ;;
    "$h"/*) printf '~/%s\n' "${1#"$h"/}" ;;
    *) printf '%s\n' "$1" ;;
  esac
}

# Each selected name without its trailing `/`; `../` is never acted on.
selected() {
  printf '%s\n' "$IT" | while IFS= read -r n; do
    n=${n%/}
    [ -n "$n" ] && [ "$n" != ".." ] && printf '%s\n' "$n"
  done
}

action=$1
case "$action" in
  copy | move | delete)
    [ -n "$(selected)" ] || { echo "nothing selected (../ is skipped)" >&2; exit 1; } ;;
esac
case "$action" in
  list)
    # `../` first, then directories, then files; hidden ones with HIDDEN=true.
    d=$(absdir "$2")
    [ "$d" = / ] || echo "../"
    if [ "$HIDDEN" = true ]; then flags=-1Ap; else flags=-1p; fi
    ls $flags -- "$d" 2>/dev/null | grep '/$'
    ls $flags -- "$d" 2>/dev/null | grep -v '/$'
    ;;
  open)
    # Prints the directory to show next: the chosen one, or DIR unchanged.
    d=$(absdir "$DIR")
    case "$IT" in
      *"
"*) next=$d ;;  # several marked rows: stay
      ../) next=$(dirname -- "$d") ;;
      */)
        if [ -r "$d/$IT" ] && [ -x "$d/$IT" ]; then
          next=$(cd -- "$d/$IT" && pwd -P)
        else
          echo "cannot open ${IT%/}: permission denied" >&2
          next=$d
        fi
        ;;
      *) next=$d ;;
    esac
    tilde "$next"
    ;;
  copy | move)
    src=$(absdir "$SRC"); dst=$(absdir "$DST")
    [ "$src" = "$dst" ] && { echo "source and target are the same directory" >&2; exit 1; }
    status=0
    selected | {
      while IFS= read -r n; do
        if [ "$action" = copy ]; then
          cp -R -- "$src/$n" "$dst/" && echo "copied $n" || status=1
        else
          mv -- "$src/$n" "$dst/" && echo "moved $n" || status=1
        fi
      done
      exit $status
    }
    ;;
  delete)
    src=$(absdir "$SRC")
    selected | while IFS= read -r n; do
      rm -rf -- "${src:?}/$n" && echo "deleted $n"
    done
    ;;
  mkdir)
    src=$(absdir "$SRC")
    case "$NAME" in
      "" | */*) echo "give a name without /" >&2; exit 1 ;;
    esac
    mkdir -- "$src/$NAME" && echo "created $NAME/"
    ;;
  view | edit)
    src=$(absdir "$SRC")
    n=$(selected | head -n 1)
    [ -n "$n" ] || exit 0
    if [ -d "$src/$n" ]; then
      ls -la -- "$src/$n" | ${PAGER:-less}
    elif [ "$action" = view ]; then
      ${PAGER:-less} -- "$src/$n"
    else
      ${EDITOR:-vi} -- "$src/$n"
    fi
    ;;
  *)
    echo "fm.sh: unknown action $action" >&2
    exit 2
    ;;
esac
