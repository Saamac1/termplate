#!/bin/sh
# Demo backup: values arrive in the environment (spec §15).
echo "Backing up to $TARGET (level $LEVEL, verbose $VERBOSE)"
for i in 1 2 3; do
  echo "step $i/3"
  sleep 0.2
done
[ -d "$TARGET" ] || { echo "target $TARGET missing" >&2; exit 1; }
echo "done"
