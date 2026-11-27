#!/usr/bin/env bash
# Disassemble a range of libil2cpp.so and label calls with method names.
# Usage: tools/disasm.sh <game> <start-addr> <end-addr>   (hex VAs from dump.cs, e.g. 0x1CC6DD8)
#        GAME=<game> tools/disasm.sh <start-addr> <end-addr>
set -euo pipefail
source "$(dirname "$0")/game.sh"
LIB="$WORK/unpacked/arm64/lib/arm64-v8a/libil2cpp.so"
aarch64-linux-gnu-objdump -d --no-show-raw-insn --start-address="$1" --stop-address="$2" "$LIB" \
  | python3 -I -c '
import json, re, sys
names = {m["Address"]: m["Name"] for m in json.load(open(sys.argv[1]))["ScriptMethod"]}
for l in sys.stdin:
    m = re.search(r"\bbl?\s+([0-9a-f]+)", l)
    if m and int(m.group(1), 16) in names:
        l = l.rstrip() + "   ; " + names[int(m.group(1), 16)] + "\n"
    sys.stdout.write(l)
' "$WORK/re/il2cpp-dump/script.json"
