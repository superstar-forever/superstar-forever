#!/usr/bin/env python3
"""Find callers of IL2CPP methods.

Usage: tools/xref.py --game <id> <regex on target method name> [--all]
  --all  also show callers inside CodeStage.AntiCheat itself

Needs work/<game>/re/il2cpp-dump/script.json and re/calls.txt (tools/dump-il2cpp.sh).
"""

import bisect, json, re, sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from game import ROOT, select

game, _ = select()
methods = json.load(open(game.re_local / "il2cpp-dump/script.json"))["ScriptMethod"]
meth = sorted((m["Address"], m["Name"]) for m in methods)
addrs = [a for a, _ in meth]
name_of = {}
for a, n in meth:
    name_of.setdefault(a, n)


def owner(addr):
    i = bisect.bisect_right(addrs, addr) - 1
    return meth[i][1] if i >= 0 else "?"


pat = re.compile(sys.argv[1])
show_all = "--all" in sys.argv
targets = {a for a, n in meth if pat.search(n)}
seen = {}
for line in open(game.re_local / "calls.txt"):
    p = line.split()
    src, dst = int(p[0][:-1], 16), int(p[2], 16)
    if dst in targets:
        o = owner(src)
        if not show_all and "CodeStage.AntiCheat" in o:
            continue
        key = f"{o}  ->  {name_of[dst]}"
        seen[key] = seen.get(key, 0) + 1
for k, v in sorted(seen.items(), key=lambda kv: -kv[1]):
    print(f"{v:4}  {k}")
