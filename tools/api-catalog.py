#!/usr/bin/env python3
"""Build a catalog of the game's server API from the IL2CPP dump.

Usage: tools/api-catalog.py --game <id> [--rescan]
  (reuses work/<game>/re/catalog-raw.json unless --rescan)
Writes games/<game>/re/api/catalog.json and catalog.md.

Every call goes through Client.request(callback, ignoreAsset, clazz, method, paramz).
The command classes in com.dalcomsoft.project.client.command wrap those calls, one
static method per RPC. For each wrapper this collects:
  - the RPC class and method names (string literals passed to Client.request),
  - the C# signature (its parameters are what goes into paramz, usually in order and
    often prefixed with the user's object ID),
  - response model types (JToken.ToObject<T> / JsonConvert.DeserializeObject<T>) and
    other string literals (often JSON keys read from the result) found in the wrapper,
    its lambdas and the *Result helpers it calls.
"""

import json, re, subprocess, sys
from collections import defaultdict
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from game import ROOT, select

game, _ = select()
NS = "com.dalcomsoft.project.client.command."
DUMP = game.re_local / "il2cpp-dump/dump.cs"

methods = json.load(open(game.re_local / "il2cpp-dump/script.json"))["ScriptMethod"]
ordered = sorted((m["Address"], m["Name"]) for m in methods)
end_of = {a: ordered[i + 1][0] for i, (a, _) in enumerate(ordered[:-1])}

# C# signatures from dump.cs, keyed by "Class$$method" in file order (overloads kept apart).
sigs = defaultdict(list)
cls = None
for line in open(DUMP, encoding="utf-8"):
    m = re.match(r"public class (\w+) : ICommand", line)
    if m:
        cls = m.group(1)
        continue
    if cls and line.startswith("}"):
        cls = None
    if cls:
        m = re.match(r"\s+public static (.+?) (\w+)\((.*)\) \{ \}", line)
        if m:
            sigs[f"{cls}$${m.group(2)}"].append({"returns": m.group(1), "params": m.group(3)})


def literals(start, end):
    out = subprocess.run(
        [str(ROOT / "tools/literals.py"), "--game", game.id, hex(start), hex(end)], capture_output=True, text=True
    ).stdout
    strs, types, calls = [], [], []
    for l in out.splitlines():
        m = re.search(r' str "(.*)"$', l)
        if m:
            strs.append(json.loads(f'"{m.group(1)}"'))
        m = re.search(r"(?:ToObject|DeserializeObject)<([^>]+(?:<[^>]*>)?)>\(\)", l)
        if m and m.group(1) not in ("object",):
            types.append(m.group(1))
        m = re.search(r"call (\S+)", l)
        if m:
            calls.append(m.group(1))
    return strs, types, calls


catalog = []
by_class = defaultdict(list)
prefix = {}  # class -> namespace prefix in script.json names
for a, n in ordered:
    if n.startswith(NS) and "$$" in n and ".<" not in n.split("$$")[0]:
        c, meth = n[len(NS) :].split("$$", 1)
        if "." not in c:
            by_class[c].append((a, meth))
            prefix[c] = NS
# ICommand classes outside the command namespace (e.g. SuperstarLeague, Mission).
global_cmds = {k.split("$$")[0] for k in sigs} - set(by_class)
for a, n in ordered:
    c, _, meth = n.partition("$$")
    if c in global_cmds and meth:
        by_class[c].append((a, meth))
        prefix[c] = ""

RAW = game.re_local / "catalog-raw.json"  # cache of the slow disassembly pass
if RAW.exists() and "--rescan" not in sys.argv:
    catalog = json.load(open(RAW))
    cached = {e["command"].split(".")[0] for e in catalog}
    by_class = {c: v for c, v in by_class.items() if c not in cached}  # scan only new classes
for c, entries in sorted(by_class.items()):
    NSC = prefix[c]
    names = {m for _, m in entries}
    seen_overload = defaultdict(int)
    for a, meth in entries:
        if meth in (".ctor", ".cctor") or meth.endswith("Result"):
            continue
        # The wrapper itself + its lambdas (<meth>b__) + *Result helpers it calls.
        ranges = [(a, end_of[a])]
        for a2, n2 in ordered:
            if n2.startswith(f"{NSC}{c}.") and f"<{meth}>" in n2:
                ranges.append((a2, end_of[a2]))
        strs, types, calls = [], [], []
        for s, e in ranges:
            x = literals(s, e)
            strs += x[0]
            types += x[1]
            calls += x[2]
        for helper in {cl.split("$$")[1] for cl in calls if cl.startswith(f"{NSC}{c}$$")}:
            if helper.endswith("Result") and helper in names:
                ha = next(x for x, m in entries if m == helper)
                x = literals(ha, end_of[ha])
                strs += x[0]
                types += x[1]
        key = f"{c}$${meth}"
        i = seen_overload[key]
        seen_overload[key] += 1
        sig = sigs[key][i] if i < len(sigs[key]) else None
        uses_request = any("Client$$request" in cl for cl in calls)
        catalog.append(
            {
                "command": f"{c}.{meth}",
                "raw_strings": strs,
                "signature": sig,
                "sends_user_object_id": any("get_userObjectID" in cl for cl in calls),
                "direct_request": uses_request,
                "response_types": sorted(set(types)),
            }
        )

if by_class:
    RAW.parent.mkdir(parents=True, exist_ok=True)
    json.dump(catalog, open(RAW, "w"), ensure_ascii=False)

# Some GOT loads are mis-resolved by literals.py and show up in wrappers of many
# unrelated command classes; drop strings seen in 5+ different classes. (Real RPC class
# names like "Account" are frequent too, but only within their own class.)
seen_in = defaultdict(set)
for e in catalog:
    for s in set(e["raw_strings"]):
        seen_in[s].add(e["command"].split(".")[0])
noise = {s for s, cl in seen_in.items() if len(cl) >= 5}


def most_common(strs, pattern, own=None):
    # clazz and method are each loaded twice (log line + request); ties go to the first.
    # `own` (the wrapper's own C# name) is allowed even if it is also a noise string.
    cands = [s for s in strs if re.fullmatch(pattern, s) and (s not in noise or s == own)]
    return max(cands, key=lambda s: (cands.count(s), -cands.index(s))) if cands else None


for e in catalog:
    strs = e.pop("raw_strings")
    clazz = most_common(strs, r"[A-Z][A-Za-z0-9]*")
    method = most_common(strs, r"[a-z][A-Za-z0-9_]*", own=e["command"].split(".")[1])
    e["rpc"] = {"clazz": clazz, "method": method}
    e["strings"] = sorted(set(strs) - noise - {clazz, method})
    for k in ("signature", "sends_user_object_id", "direct_request", "response_types", "strings"):
        e[k] = e.pop(k)  # key order: command, rpc, then the rest

out = game.re / "api"
out.mkdir(parents=True, exist_ok=True)
(out / "catalog.json").write_text(
    json.dumps(catalog, indent=2, ensure_ascii=False) + "\n", encoding="utf-8"
)  # jq style

with open(out / "catalog.md", "w") as f:
    f.write(
        "# API catalog (generated by tools/api-catalog.py)\n\n"
        "RPC = `clazz.method` in the request body. Params = C# parameters of the wrapper "
        "(callback omitted); `uid` = the user's object ID is also sent.\n"
    )
    cur = None
    for e in catalog:
        c = e["command"].split(".")[0]
        if c != cur:
            cur = c
            f.write(f"\n## {c}\n\n| RPC | Params | Response types | Other strings |\n|---|---|---|---|\n")
        rpc = f"`{e['rpc']['clazz']}.{e['rpc']['method']}`" if e["rpc"]["method"] else f"({e['command']})"
        params = e["signature"]["params"] if e["signature"] else "?"
        params = re.sub(r"Action<[^>]*(?:<[^>]*>[^>]*)*> \w+(, )?", "", params).replace("|", "\\|")
        if e["sends_user_object_id"]:
            params = "uid" + (", " + params if params else "")
        strs = ", ".join(f"`{s}`" for s in e["strings"][:12])
        f.write(f"| {rpc} | {params} | {', '.join(e['response_types'])} | {strs} |\n")

print(f"{len(catalog)} commands -> games/{game.id}/re/api/catalog.json, catalog.md")
