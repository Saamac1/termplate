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


def spawn(args, cwd=None):
    master, slave = os.openpty()
    fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack("HHHH", 20, 70, 0, 0))

    def ctty():
        os.setsid()
        fcntl.ioctl(slave, termios.TIOCSCTTY, 0)

    p = subprocess.Popen([BIN] + args, stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
                         preexec_fn=ctty, cwd=cwd or os.path.dirname(APP), env={**os.environ, "LANG": "en_US.UTF-8"})
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
    sys.exit(0 if ok else 1)


if __name__ == "__main__":
    main()
