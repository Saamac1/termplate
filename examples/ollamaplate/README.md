# Ollamaplate

A terminal UI for [Ollama](https://ollama.com): start the server, choose a
model, see which models are loaded, and chat.

```sh
termplate run examples/ollamaplate/app.termplate
```

Needs `ollama`, `curl` and `python3` (standard library only).

| Area (key) | What it does |
|---|---|
| Ollama (F1) | Start/stop `ollama serve` (Start is disabled when a server already answers, e.g. the Ollama app), new chat, unload the model |
| Model (F2) | `ollama list`; Enter chooses the model for the chat |
| Loaded | `ollama ps`, refreshed every 3 seconds |
| Chat (F3) | the conversation; the reply appears as it is generated |
| Server (F4) | the log of the server started from here |
| Ask (F5) | type and press Enter to send |

Ctrl+N starts a new chat, Ctrl+R refreshes, Ctrl+Q quits. `OLLAMA_HOST`
is respected, so `OLLAMA_HOST=127.0.0.1:11999 termplate run …` starts and
uses a server of its own.

Files:

- `chat.py` talks to `/api/chat`, keeps the conversation in
  `$TMPDIR/ollamaplate-history.json` and prints the reply wrapped at
  `CHAT_WIDTH` columns, so the chat log can show it line by line.
- `ollama.sh` answers the menus and the panel. It checks first that a
  server answers, because `ollama list` and `ollama ps` retry for seconds
  when none does, and Termplate still waits for panel values.
