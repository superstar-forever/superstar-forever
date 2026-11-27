#!/usr/bin/env bash
# Disassemble libil2cpp.so and keep every direct branch/call (b / bl) for xref.py.
# Usage: tools/index-calls.sh <game>        -> work/<game>/re/calls.txt
set -euo pipefail
source "$(dirname "$0")/game.sh"
LIB="$WORK/unpacked/arm64/lib/arm64-v8a/libil2cpp.so"
mkdir -p "$WORK/re"
aarch64-linux-gnu-objdump -d --no-show-raw-insn "$LIB" 2>/dev/null \
  | grep -E '^\s+[0-9a-f]+:\s+bl?\s+[0-9a-f]+' > "$WORK/re/calls.txt"
echo "Wrote $(wc -l < "$WORK/re/calls.txt") calls to work/$GAME/re/calls.txt"
