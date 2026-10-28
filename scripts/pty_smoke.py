#!/usr/bin/env python3
"""End-to-end smoke test of the terminal paths that unit tests cannot reach.

Runs the built binary with a pseudo-terminal as its controlling tty (keys
and drawing go through /dev/tty) while stdin/stdout stay pipes, like in
`choice=$(termplate menu …)` and `coproc termplate serve …`.

    cargo build && python3 scripts/pty_smoke.py
"""
import fcntl, os, struct, subprocess, sys, termios, threading, time

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
BIN = os.path.join(ROOT, "target", "debug", "termplate")
APP = os.path.join(ROOT, "examples", "backup", "app.termplate")


SLOW_APP = """app SLOW
- termplate 0.1

layout MAIN
mmll

legend:
- m MAIN_MENU
- l LOG

menu MAIN_MENU
- SLOW
- QUIT

command SLOW
- run echo started; sleep 5; echo finished
- output LOG

log LOG history 50
"""


SLOW_PANEL_APP = """app SLOWPANEL
- termplate 0.1

layout MAIN
ppmm

legend:
- p CLOCK
- m MAIN_MENU

panel CLOCK refresh 1s
vvvvvvvvvv

legend:
- v value $(sleep 2; echo ticked)

menu MAIN_MENU
- FIRST open OTHER
- SECOND open OTHER
- QUIT quit

menu OTHER
- BACK back
"""


FORM_UI = """vars
- HOST text "localhost"
- PORT number 22 range 1..65535

panel LOGIN
hhhh [host__________]
pppp [port_]

legend:
- h text LOGIN.HOST "Host"
- p text LOGIN.PORT "Port"
- host input HOST
- port input PORT
"""


SCRIPT_APP = """app SCRIPTED
- termplate 0.1

vars
- NAME text "world"

layout MAIN
mmll

legend:
- m MAIN_MENU
- l LOG

menu MAIN_MENU
- GREET
- MONITOR
- QUIT

command GREET
- env NAME=$NAME
- output LOG
run:
```bash
# a comment, and a > redirection
greeting="hello"
echo "$greeting, ${NAME}!" > /dev/stdout
echo "bash ${BASH_VERSION%%.*}"
```

command MONITOR
- env NAME=$NAME
- window split
run:
```
echo "window: $NAME" > window.out
```

log LOG history 50
"""

# Stands in for tmux: records its arguments and runs the window's command.
FAKE_TMUX = """#!/bin/sh
if [ "$1" = "-V" ]; then echo "tmux 3.3a"; exit 0; fi
echo "$@" > tmux.args
for a; do last=$a; done
sh -c "$last" &
"""


def spawn(args, cwd=None, stdin_data=None, env=None):
    master, slave = os.openpty()
    fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack("HHHH", 20, 70, 0, 0))

    def ctty():
        os.setsid()
        fcntl.ioctl(slave, termios.TIOCSCTTY, 0)

    p = subprocess.Popen([BIN] + args, stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
                         preexec_fn=ctty, cwd=cwd or os.path.dirname(APP), env={**os.environ, "LANG": "en_US.UTF-8", **(env or {})})
    if stdin_data is not None:
        p.stdin.write(stdin_data.encode())
        p.stdin.close()
    screen = bytearray()

    def drain():
        while True:
            try:
                data = os.read(master, 65536)
            except OSError:
                break
            if not data:
                break
            screen.extend(data)

    threading.Thread(target=drain, daemon=True).start()
    time.sleep(0.8)
    return p, master, screen


def finish(p):
    try:
        if p.stdin.closed:
            p.stdin = None
        out, err = p.communicate(timeout=5)
    except subprocess.TimeoutExpired:
        p.kill()
        out, err = p.communicate()
        raise SystemExit("timed out")
    return p.returncode, out.decode(), err.decode()


def check(name, cond, detail):
    print(("ok   " if cond else "FAIL ") + name)
    if not cond:
        print("     " + repr(detail))
    return cond


def main():
    ok = True
    # widget mode: j moves down (RESTORE), Enter chooses
    p, tty, screen = spawn(["menu", APP, "MAIN_MENU"])
    os.write(tty, b"j"); time.sleep(0.3)
    os.write(tty, b"\r")
    code, out, _ = finish(p)
    ok &= check("menu prints choice", (code, out) == (0, "RESTORE\n"), (code, out))
    ok &= check("menu drew items", b"Restore" in screen, bytes(screen[-300:]))

    # widget mode: Esc is `back` -> exit 1
    p, tty, _ = spawn(["menu", APP, "MAIN_MENU"])
    os.write(tty, b"\x1b")
    code, out, _ = finish(p)
    ok &= check("menu back exits 1", (code, out) == (1, ""), (code, out))

    # coprocess mode
    p, tty, _ = spawn(["serve", APP, "MAIN_MENU"])
    p.stdin.write(b"set TARGET /var\nbogus\n"); p.stdin.flush(); time.sleep(0.3)
    os.write(tty, b"\r"); time.sleep(0.4)
    os.write(tty, b"\x1b"); time.sleep(0.6)  # a lone Esc needs a moment to be told from a sequence
    code, out, _ = finish(p)
    expected = "error protocol unknown message `bogus`\nselect MAIN_MENU BACKUP\nclosed MAIN_MENU\n"
    ok &= check("serve events", (code, out) == (0, expected), (code, out))

    # background command: the UI keeps running, Esc cancels it
    import tempfile
    with tempfile.TemporaryDirectory() as d:
        app = os.path.join(d, "slow.termplate")
        with open(app, "w") as f:
            f.write(SLOW_APP)
        p, tty, screen = spawn(["run", app], cwd=d)
        started = time.time()
        os.write(tty, b"\r"); time.sleep(0.6)
        ok &= check("background output streams", b"started" in screen and b"finished" not in screen, bytes(screen[-400:]))
        os.write(tty, b"\x1b"); time.sleep(0.8)
        ok &= check("esc cancels the command", b"SLOW cancelled" in screen, bytes(screen[-400:]))
        os.write(tty, b"\x11")  # ctrl+q
        code, _, _ = finish(p)
        ok &= check("quit while idle", code == 0 and time.time() - started < 4, (code, time.time() - started))

        # quitting while a command runs stops the command
        p, tty, screen = spawn(["run", app], cwd=d)
        os.write(tty, b"\r"); time.sleep(0.5)
        os.write(tty, b"\x11")
        code, _, _ = finish(p)
        time.sleep(0.2)
        left = subprocess.run(["pgrep", "-f", "echo started; sleep 5"], capture_output=True).stdout
        ok &= check("quit stops running commands", code == 0 and not left, (code, left))

        # a slow panel value does not hold up drawing or keys
        app = os.path.join(d, "slowpanel.termplate")
        with open(app, "w") as f:
            f.write(SLOW_PANEL_APP)
        p, tty, screen = spawn(["run", app], cwd=d)
        ok &= check("drawn before the slow panel value", b"SECOND" in screen and b"ticked" not in screen, bytes(screen[-300:]))
        os.write(tty, b"j"); time.sleep(0.3)
        os.write(tty, b"\r"); time.sleep(0.2)
        ok &= check("keys work while the panel waits", b"BACK" in screen and b"ticked" not in screen, bytes(screen[-300:]))
        time.sleep(2.0)
        ok &= check("panel value arrives", b"ticked" in screen, bytes(screen[-300:]))
        os.write(tty, b"\x11")
        code, _, _ = finish(p)
        ok &= check("quit with a panel evaluation running", code == 0, code)

        # a form in widget mode: Tab between fields, Enter prints the values
        ui = os.path.join(d, "form.termplate")
        with open(ui, "w") as f:
            f.write(FORM_UI)
        p, tty, screen = spawn(["panel", ui, "LOGIN"], cwd=d)
        os.write(tty, b"\x7f" * 9 + b"example.org\t"); time.sleep(0.3)
        os.write(tty, b"\x7f\x7f2222"); time.sleep(0.3)
        os.write(tty, b"\r")
        code, out, _ = finish(p)
        ok &= check("form widget prints values", (code, out) == (0, "example.org\n2222\n"), (code, out))

        # the Termplate file from stdin, as in a heredoc
        menu = 'menu MAIN_MENU\n- BACKUP "Back up"\n- RESTORE "Restore"\n'
        p, tty, screen = spawn(["menu", "-", "MAIN_MENU"], cwd=d, stdin_data=menu)
        os.write(tty, b"j"); time.sleep(0.3)
        os.write(tty, b"\r")
        code, out, err = finish(p)
        ok &= check("menu from stdin", (code, out) == (0, "RESTORE\n"), (code, out, err))

        # an embedded bash script, and a window command through tmux
        app = os.path.join(d, "scripted.termplate")
        with open(app, "w") as f:
            f.write(SCRIPT_APP)
        bindir = os.path.join(d, "bin")
        os.mkdir(bindir)
        with open(os.path.join(bindir, "tmux"), "w") as f:
            f.write(FAKE_TMUX)
        os.chmod(os.path.join(bindir, "tmux"), 0o755)
        env = {"TMUX": "/tmp/fake,1,0", "TMUX_PANE": "%7", "PATH": bindir + ":" + os.environ["PATH"]}
        p, tty, screen = spawn(["run", app], cwd=d, env=env)
        os.write(tty, b"\r"); time.sleep(0.8)
        ok &= check("script output in the log", b"hello, world!" in screen and b"bash " in screen, bytes(screen[-400:]))
        os.write(tty, b"j"); time.sleep(0.2)
        os.write(tty, b"\r"); time.sleep(0.8)
        args = open(os.path.join(d, "tmux.args")).read() if os.path.exists(os.path.join(d, "tmux.args")) else ""
        ok &= check("window opened with split-window", args.startswith("split-window -t %7 -c "), args)
        out = open(os.path.join(d, "window.out")).read() if os.path.exists(os.path.join(d, "window.out")) else ""
        ok &= check("window command got its env", out == "window: world\n", out)
        leftovers = [n for n in os.listdir(tempfile.gettempdir()) if n.startswith("termplate-%d-" % p.pid)]
        ok &= check("temporary files removed", not leftovers, leftovers)
        os.write(tty, b"\x11")
        code, _, _ = finish(p)
        ok &= check("quit after scripts", code == 0, code)
    sys.exit(0 if ok else 1)


if __name__ == "__main__":
    main()
