#!/usr/bin/env python3
"""Assemble work/<game>/server-data/, everything the Rust server reads at runtime.

Usage: tools/build-server-data.py --game <id> [--copy]

work/<game>/server-data/
  cdn/<host>/<path>      every game file, laid out exactly like the original CDN URLs
                         (hardlinks into cdn-mirror/<game>/; --copy copies instead,
                         e.g. when preparing a folder for a phone on another filesystem),
                         plus cdn/offline.local/news/offline-edition.png, the lobby news banner
                         (tools/news_banner.py, drawn from the game's title logo)
  asset-context.json     master data index the game asks for (a.json: version + table URLs)
  manifest.json          startup manifest (games/<game>/manifest.json; ServerUrl filled in by the server)
  progression.json       card progression settings (copy of games/<game>/progression.json)
  server.json            {"port": N}: where the server inside the app listens (game.json "port")
  game.key               the game's AES key, read from your libssnative.so (tools/gamekey.py)
  api-defaults.json      "Class.method" -> default `result` for every known API call, built from
                         the response model types (empty lists, zeros, nested default objects)

Needs: games/<game>/re/api/catalog.json and models.json (tools/api-catalog.py, tools/models.py),
the game's asset context (game.json "assetContext"), cdn-mirror/<game>/ and work/<game>/unpacked/.
"""

import json, os, re, shutil, sys
from urllib.parse import urlsplit
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from game import ROOT, select

game, _ = select()
OUT = game.server_data
ASSET_CONTEXT = game.asset_context  # table index (a.json) as the game got it
COPY = "--copy" in sys.argv


def place(src: Path, rel: Path, stats):
    dst = OUT / "cdn" / rel
    if dst.exists():
        stats["skipped"] += 1
        return
    dst.parent.mkdir(parents=True, exist_ok=True)
    if COPY:
        shutil.copy2(src, dst)
    else:
        try:
            os.link(src, dst)
        except OSError:
            shutil.copy2(src, dst)
    stats["placed"] += 1


def build_cdn():
    """cdn-mirror/<game>/ holds the game files in URL layout. With a download list
    (games/<game>/cdn-files.aria2, tools/make-aria2.py), only the files in it are placed: the mirror
    can hold more (older table versions, content of removed shop pages and hidden menus).
    Without one (a game whose mirror comes from a device cache, tools/import-device-cache.py),
    every file in the mirror is placed."""
    stats = {"placed": 0, "skipped": 0, "missing": 0}
    shutil.rmtree(OUT / "cdn", ignore_errors=True)  # hardlinks: cheap to recreate, drops files no longer listed
    mirror = game.mirror
    if not game.file("cdn-files.aria2").exists():
        for src in sorted(mirror.rglob("*")):
            if src.is_file() and src.name not in ("CACHEDIR.TAG", ".gitkeep"):
                place(src, src.relative_to(mirror), stats)
        return stats
    prefix = f"cdn-mirror/{game.id}/"
    d = None
    for line in open(game.file("cdn-files.aria2"), encoding="utf-8"):
        line = line.strip()
        if line.startswith("dir="):
            d = line[4:]
        elif line.startswith("out=") and d and d.startswith(prefix):
            rel = Path(d[len(prefix) :]) / line[4:]
            if (mirror / rel).is_file():
                place(mirror / rel, rel, stats)
            else:
                stats["missing"] += 1
    return stats


# ---------------------------------------------------------------- default API responses

models = json.load(open(game.re / "api/models.json"))
PRIMITIVE = {
    "int": 0,
    "long": 0,
    "short": 0,
    "byte": 0,
    "sbyte": 0,
    "uint": 0,
    "ulong": 0,
    "ushort": 0,
    "float": 0.0,
    "double": 0.0,
    "decimal": 0,
    "bool": False,
    "string": "",
    "object": None,
    "DateTime": None,
    "JObject": {},
    "JArray": [],
    "JToken": None,
}


def default_for(typ, stack=()):
    typ = typ.strip()
    if typ in PRIMITIVE:
        return PRIMITIVE[typ]
    if typ.endswith("[]") or re.match(r"(List|IList|HashSet|IEnumerable)<", typ):
        return []
    if re.match(r"(Dictionary|IDictionary)<", typ):
        return {}
    if typ.startswith("Nullable<"):
        return None
    m = models.get(typ) or models.get(typ.split(".")[-1])
    if m is None or typ in stack or len(stack) > 6:
        return None
    if m["kind"] == "enum":
        vals = [int(f["value"]) for f in m["fields"] if re.fullmatch(r"-?\d+", f.get("value") or "")]
        return 0 if 0 in vals or not vals else vals[0]
    obj = {}
    base = m.get("base")
    if base and base in models and models[base]["kind"] != "enum":
        obj.update(default_for(base, stack + (typ,)) or {})
    for f in m["fields"]:
        if "type" in f:
            obj[f.get("json") or f["name"]] = default_for(f["type"], stack + (typ,))
    return obj


def build_defaults():
    out = {}
    for e in json.load(open(game.re / "api/catalog.json")):
        if e["command"].startswith("DalcomAccount."):
            continue
        # The catalog doesn't always find the strings the request is built from; the
        # command name (class.method of the client API) is the same in every case seen so far.
        cmd_clazz, _, cmd_method = e["command"].partition(".")
        clazz = e["rpc"]["clazz"] or (cmd_clazz if e["direct_request"] else None)
        method = e["rpc"]["method"] or (cmd_method if e["direct_request"] else None)
        if not clazz or not method:
            continue
        types = [t for t in e["response_types"] if t not in ("int", "object")]
        # SSGF's response handlers also read the arena info that rides along with many
        # responses; the call's own result type is the other one.
        if len(types) > 2 and types[:2] == ["ArenaBasicInfo", "ArenaLeagueReward"]:
            types = types[2:]
        key = f"{clazz}.{method}"
        val = default_for(types[0]) if types else None
        # Overloads/duplicates: keep the richer default.
        if key not in out or (out[key] is None and val is not None):
            out[key] = val
    overrides = json.load(open(game.file("api-overrides.json"), encoding="utf-8"))
    out.update({k: v for k, v in overrides.items() if not k.startswith("_")})
    return out


def apply_table_overrides(asset_context):
    """games/<game>/table-overrides.json: modify master data tables. The changed table is written
    as a new file in server-data/cdn (the hardlink to cdn-mirror/ is replaced, so the mirror
    copy stays original) and gets a higher version in the asset context, so installed games
    download it again. Versions start at original + 1000 and go up by one whenever the
    overridden content changes (games/<game>/table-versions.json remembers content hash and version;
    keep it in git)."""
    import hashlib

    vfile = game.file("table-versions.json")
    versions = json.load(open(vfile)) if vfile.exists() else {}
    import tables

    dm = tables.load_decrypt()

    for table, table_rules in tables.load_rules().items():
        entry = asset_context["context"][table]
        rel = tables.rel_path(entry["file"])
        src = game.mirror / rel
        raw = src.read_bytes()
        rows, changed, removed = tables.apply_rules(json.loads(dm.decrypt(raw)), table_rules)
        dst = OUT / "cdn" / rel
        dst.unlink(missing_ok=True)  # never write through the hardlink into cdn-mirror/
        dst.parent.mkdir(parents=True, exist_ok=True)
        plain = json.dumps(rows, ensure_ascii=False, separators=(",", ":")).encode()
        dst.write_bytes(dm.encrypt(plain, compress=raw[:2] == b"\x1f\x8b"))
        digest = hashlib.sha256(plain).hexdigest()[:16]
        prev = versions.get(table, {})
        base = int(entry["version"]) + 1000
        if prev.get("hash") == digest:
            version = prev["version"]
        else:
            version = max(base, prev.get("version", base - 1) + 1)
        versions[table] = {"hash": digest, "version": version}
        entry["version"] = version
        print(f"table override: {table}: {changed} row(s) changed, {removed} removed, version -> {version}")
    vfile.write_text(json.dumps(versions, indent=2, sort_keys=True, ensure_ascii=False) + "\n", encoding="utf-8")


def add_empty_tables(asset_context):
    """Tables in the asset context whose file is gone (e.g. LOONA's AssetPackData on the dead CDN) are served
    as an encrypted empty list. A game that downloads such a table would otherwise get the server's 404 page
    and fail to decrypt it (JsonParseError at the title screen). The version goes up by 1000 (like an override),
    so a game that cached the 404 page downloads the table again. Entries without a file name (CdnAddress)
    are never downloaded and stay as they are."""
    import tables

    dm = tables.load_decrypt()
    for table, entry in asset_context["context"].items():
        rel = tables.rel_path(entry["file"])
        dst = OUT / "cdn" / rel
        if entry["file"].endswith("/") or dst.exists():
            continue
        dst.parent.mkdir(parents=True, exist_ok=True)
        dst.write_bytes(dm.encrypt(b"[]", compress=False))
        entry["version"] = int(entry["version"]) + 1000
        print(f"empty table: {table} (no file in cdn-mirror/), version -> {entry['version']}")


def main():
    OUT.mkdir(parents=True, exist_ok=True)
    stats = build_cdn()
    print(
        f"cdn/: {stats['placed']} files placed, {stats['skipped']} already present"
        + (f", {stats['missing']} listed but not in cdn-mirror/ (download them first)" if stats["missing"] else "")
    )
    from news_banner import draw as draw_news_banner

    draw_news_banner(game, OUT / "cdn/offline.local/news/offline-edition.png")

    a = json.load(open(ASSET_CONTEXT, encoding="utf-8-sig"))
    apply_table_overrides(a)
    add_empty_tables(a)
    json.dump(a, open(OUT / "asset-context.json", "w"), indent=1)
    manifest = json.load(open(game.file("manifest.json")))
    manifest.pop("_comment", None)
    json.dump(manifest, open(OUT / "manifest.json", "w"), indent=1)
    shutil.copy(game.file("progression.json"), OUT / "progression.json")
    json.dump({"port": game.cfg.get("port", 18384)}, open(OUT / "server.json", "w"))
    from gamekey import game_key

    (OUT / "game.key").write_bytes(game_key())  # from your copy of the game, see tools/gamekey.py

    defaults = build_defaults()
    json.dump(defaults, open(OUT / "api-defaults.json", "w"), indent=1, ensure_ascii=False)
    print(f"api-defaults.json: {len(defaults)} calls")


if __name__ == "__main__":
    main()
