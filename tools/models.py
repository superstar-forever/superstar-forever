#!/usr/bin/env python3
"""Export the API data model classes from the IL2CPP dump.

Usage: tools/models.py --game <id>
Writes games/<game>/re/api/models.json: for every class/enum in com.dalcomsoft.project.client.model.*
(and nested response classes of the command classes), its base class and its instance
fields as {name, type, json}. `json` is the [JsonProperty] name when one is set.
Static fields, consts and backing fields of computed properties are skipped; for
auto-properties the property name is used.
"""

import json, re, sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from game import ROOT, select

game, _ = select()
DUMP = game.re_local / "il2cpp-dump/dump.cs"

NS_RE = re.compile(r"// Namespace: (.*)")
CLASS_RE = re.compile(
    r"^(?:public|internal|private|protected)[\w ]*? (class|struct|enum) ([\w.<>`, ]+?)"
    r"(?: : ([\w.<>, ]+))? // TypeDefIndex"
)
FIELD_RE = re.compile(
    r"^\t(?:public|private|protected|internal)( static| const)?( readonly)? "
    r"(.+?) (<?\w+>?(?:k__BackingField)?)(?: = (.*?))?;(?: // (0x[0-9A-F]+))?$"
)
PROP_RE = re.compile(r"^\tpublic (?:virtual |override )?(.+?) (\w+) \{ get; (?:set; )?\}")
ATTR_RE = re.compile(r'^\t\[JsonProperty\((?:PropertyName = )?"([^"]+)"')

models = {}
ns = cur = None
pending_json = None
for line in open(DUMP, encoding="utf-8"):
    m = NS_RE.match(line)
    if m:
        ns = m.group(1)
        continue
    m = CLASS_RE.match(line)
    if m:
        kind, name, base = m.groups()
        wanted = ns.startswith("com.dalcomsoft.project.client.model") or (
            ns == ""
            and re.match(
                r"(Game|Store|Account|CardInventory|CardBook|Arena|Attendance|"
                r"LeagueWeekly|LiveTheme|MusicBattle|Platform|PostBox|Promotion|"
                r"StarPass)\.\w+",
                name,
            )
        )
        cur = None
        if wanted:
            full = f"{ns}.{name}" if ns else name
            cur = models[name] = {"namespace": ns, "kind": kind, "base": base, "fields": []}
        continue
    if cur is None:
        continue
    if line.startswith("}"):
        cur = None
        continue
    m = ATTR_RE.match(line)
    if m:
        pending_json = m.group(1)
        continue
    m = FIELD_RE.match(line)
    if m:
        static, _, typ, fname, value, _ = m.groups()
        if static and "const" in static:
            # Enum members, and constants of static code classes such as ResponseCode.
            cur["fields"].append({"name": fname, "value": value})
        elif not static:
            auto = re.fullmatch(r"<(\w+)>k__BackingField", fname)
            cur["fields"].append(
                {
                    "name": auto.group(1) if auto else fname,
                    "type": typ,
                    **({"json": pending_json} if pending_json else {}),
                }
            )
        pending_json = None
        continue
    m = PROP_RE.match(line)
    if m and pending_json:
        # [JsonProperty] on an auto-property: attach the name to its backing field.
        for f in cur["fields"]:
            if f["name"] == m.group(2):
                f["json"] = pending_json
        pending_json = None
    elif not line.startswith("\t["):
        pending_json = None

out = game.re / "api"
out.mkdir(parents=True, exist_ok=True)
(out / "models.json").write_text(json.dumps(models, indent=2, ensure_ascii=False) + "\n", encoding="utf-8")  # jq style
print(
    f"{len(models)} models -> games/{game.id}/re/api/models.json "
    f"({sum(1 for m in models.values() if any('json' in f for f in m['fields']))} use [JsonProperty])"
)
