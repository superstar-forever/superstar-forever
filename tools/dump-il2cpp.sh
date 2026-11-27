#!/usr/bin/env bash
# Run Il2CppDumper on the unpacked game, then build the call index used by xref.py.
# Usage: tools/dump-il2cpp.sh <game>        -> work/<game>/re/il2cpp-dump/, re/calls.txt
set -euo pipefail
source "$(dirname "$0")/game.sh"
LIB="$WORK/unpacked/arm64/lib/arm64-v8a/libil2cpp.so"
META="$WORK/unpacked/base/assets/bin/Data/Managed/Metadata/global-metadata.dat"
OUT="$WORK/re/il2cpp-dump"
mkdir -p "$OUT"
il2cppdumper --no-require-any-key --dump-attribute --dump-property "$LIB" "$META" "$OUT"
GAME="$GAME" "$ROOT/tools/index-calls.sh"
