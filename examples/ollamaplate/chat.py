#!/usr/bin/env python3
"""Chat helper for Ollamaplate: talks to the Ollama HTTP API.

    chat.py ask     MODEL and PROMPT come in the environment (spec §6)
    chat.py reset   forgets the conversation

The reply is printed as it arrives, wrapped at WIDTH columns (default 70)
so that Termplate, which shows whole lines, can show it while it grows.
The conversation is kept in $TMPDIR/ollamaplate-history.json.
"""
import json
import os
import sys
import tempfile
import urllib.error
import urllib.request

HISTORY = os.path.join(tempfile.gettempdir(), "ollamaplate-history.json")


def base_url():
    host = os.environ.get("OLLAMA_HOST") or "127.0.0.1:11434"
    return host if host.startswith("http") else "http://" + host


def load_history():
    try:
        with open(HISTORY) as f:
            return json.load(f)
    except (OSError, ValueError):
        return []


class Wrapper:
    """Prints text in lines of at most `width` columns, breaking at spaces."""

    def __init__(self, width):
        self.width = width
        self.line = ""

    def feed(self, text):
        for ch in text:
            if ch == "\n":
                self.emit(self.line)
                self.line = ""
                continue
            self.line += ch
            if len(self.line) > self.width:
                cut = self.line.rfind(" ", 0, self.width + 1)
                if cut <= 0:
                    cut = self.width
                self.emit(self.line[:cut])
                self.line = self.line[cut:].lstrip(" ")

    def close(self):
        if self.line:
            self.emit(self.line)
            self.line = ""

    @staticmethod
    def emit(line):
        print(line, flush=True)


def chat(model, messages, think):
    body = {"model": model, "messages": messages, "stream": True}
    if think is not None:
        body["think"] = think
    req = urllib.request.Request(base_url() + "/api/chat", data=json.dumps(body).encode(),
                                 headers={"Content-Type": "application/json"})
    return urllib.request.urlopen(req, timeout=600)


def ask():
    model = os.environ.get("MODEL", "")
    prompt = os.environ.get("PROMPT", "").strip()
    if not model:
        print("Choose a model first.", file=sys.stderr)
        return 1
    if not prompt:
        return 0
    print("> " + prompt, flush=True)
    messages = load_history() + [{"role": "user", "content": prompt}]
    try:
        try:
            resp = chat(model, messages, False)
        except urllib.error.HTTPError as e:
            # models without thinking support reject the `think` field
            if e.code != 400:
                raise
            resp = chat(model, messages, None)
    except urllib.error.HTTPError as e:
        body = e.read().decode(errors="replace").strip()
        try:
            body = json.loads(body).get("error", body)
        except ValueError:
            pass
        print("Ollama: " + body, file=sys.stderr)
        return 1
    except OSError as e:
        print(f"Cannot reach Ollama at {base_url()}: {e}. Is the server running?", file=sys.stderr)
        return 1
    out = Wrapper(int(os.environ.get("WIDTH") or 70))
    reply = ""
    print(model + ":", flush=True)
    for raw in resp:
        part = json.loads(raw)
        if part.get("error"):
            out.close()
            print("Ollama: " + part["error"], file=sys.stderr)
            return 1
        text = part.get("message", {}).get("content", "")
        reply += text
        out.feed(text)
        if part.get("done"):
            break
    out.close()
    print("", flush=True)
    messages.append({"role": "assistant", "content": reply})
    with open(HISTORY, "w") as f:
        json.dump(messages, f)
    return 0


def reset():
    try:
        os.remove(HISTORY)
    except OSError:
        pass
    print("--- new chat ---", flush=True)
    return 0


if __name__ == "__main__":
    action = sys.argv[1] if len(sys.argv) > 1 else "ask"
    sys.exit({"ask": ask, "reset": reset}.get(action, lambda: 2)())
