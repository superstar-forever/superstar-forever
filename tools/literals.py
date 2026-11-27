#!/usr/bin/env python3
"""Disassemble a range of libil2cpp.so and annotate string literals, types and calls.

Usage: tools/literals.py --game <id> <start-va> <end-va> [--all]
Without --all only lines with an annotation are printed.
Resolves `adrp xN, page; ldr xM, [xN, #off]` loads through the GOT (R_AARCH64_RELATIVE)
to Il2CppDumper's ScriptString / ScriptMetadata / ScriptMethod entries.
"""

import json, re, subprocess, sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from game import ROOT, select

game, _ = select()
LIB = game.libil2cpp
s = json.load(open(game.re_local / "il2cpp-dump/script.json"))
sym = {}
for x in s["ScriptString"]:
    sym[x["Address"]] = "str " + json.dumps(x["Value"], ensure_ascii=False)
for x in s["ScriptMetadata"]:
    sym[x["Address"]] = x["Name"]
for x in s["ScriptMetadataMethod"]:
    sym[x["Address"]] = "method " + x["Name"]
methods = {x["Address"]: x["Name"] for x in s["ScriptMethod"]}

cache = game.re_local / "relocs.json"
if cache.exists():
    rel = {int(k): v for k, v in json.load(open(cache)).items()}
else:
    rel = {}
    out = subprocess.run(["readelf", "-rW", str(LIB)], capture_output=True, text=True).stdout
    for l in out.splitlines():
        p = l.split()
        if len(p) >= 4 and p[2] == "R_AARCH64_RELATIVE":
            rel[int(p[0], 16)] = int(p[3], 16)
    json.dump(rel, open(cache, "w"))

start, end = sys.argv[1], sys.argv[2]
asm = subprocess.run(
    [
        "aarch64-linux-gnu-objdump",
        "-d",
        "--no-show-raw-insn",
        f"--start-address={start}",
        f"--stop-address={end}",
        str(LIB),
    ],
    capture_output=True,
    text=True,
).stdout
page = {}
for l in asm.splitlines():
    note = ""
    m = re.search(r"adrp\s+(x\d+),\s*([0-9a-f]+)", l)
    if m:
        page[m.group(1)] = int(m.group(2), 16)
    m = re.search(r"ldr\s+x\d+,\s*\[(x\d+),\s*#(\d+)\]", l)
    if m and m.group(1) in page:
        got = page[m.group(1)] + int(m.group(2))
        tgt = rel.get(got)
        if tgt in sym:
            note = sym[tgt]
    m = re.search(r"\bbl?\s+([0-9a-f]+)", l)
    if m and int(m.group(1), 16) in methods:
        note = "call " + methods[int(m.group(1), 16)]
    if note:
        print(f"{l.split(':')[0].strip()}  {note}")
    elif "--all" in sys.argv:
        print(l)
