#!/bin/sh
# Quick queries for Ollamaplate's menus and panels. They run while the UI
# waits, so they first check that a server answers: `ollama list` and
# `ollama ps` retry for several seconds when none does.

host=${OLLAMA_HOST:-127.0.0.1:11434}
case "$host" in
  http*) ;;
  *) host=http://$host ;;
esac

up() {
  curl -s -m 1 -o /dev/null "$host/api/version"
}

case "$1" in
  up) up ;;
  list) up && ollama list | awk 'NR > 1 {print $1}' ;;
  ps) up && ollama ps | awk 'NR > 1 {print $1; print "  " $3 " " $4 ", " $5 " " $6}' ;;
  *) echo "ollama.sh: unknown query $1" >&2; exit 2 ;;
esac
