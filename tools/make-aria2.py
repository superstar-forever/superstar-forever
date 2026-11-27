#!/usr/bin/env python3
"""Regenerate games/<game>/cdn-files.aria2, the download list of every CDN file the offline build
needs (maintainer tool; users just run aria2c on the shipped list, from the repository root):

  aria2c -i games/<game>/cdn-files.aria2 -c -j 8 --auto-file-renaming=false

Usage: tools/make-aria2.py --game <id>
Files land in cdn-mirror/<game>/<host>/<path>, the layout tools/build-server-data.py expects.

The list is every file in your complete cdn-mirror/ (so each was downloadable), minus files
only removed content uses: rows removed by games/<game>/table-overrides.json, store rows on
removed store tabs (storeMenuID), and the language variants (_enUS etc.) of those files.
Tables of other index versions (in the same folder as the current ones) are left out too.

Download URLs: tables use the URL from the index as it is (SSGF's contain "asset//", which S3
treats as a different key than "asset/"); hosts listed in game.json "cdnSources" (dead host ->
host that still serves the same files) are downloaded from the other host into the original
folder; non-ASCII file names are percent-encoded.

Needs cdn-mirror/<game>/ and the game's asset context (the table index).
"""

import json, re, sys
from urllib.parse import quote
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from game import ROOT, select
import tables

game, _ = select()
MIRROR = game.mirror
ASSET_CONTEXT = game.asset_context
LIST = game.file("cdn-files.aria2")
URL_RE = re.compile(r"https?://[^\s\"\\,\]\)]+")
PATH_SAFE = "/!$&'()*+,;=:@~"  # characters valid in a URL path; others are percent-encoded
LOCALE_SUFFIX = re.compile(r"_(koKR|enUS|jpJP|zhCN|zhTW|thTH|idID|esES|ptPT)(?=\.[^./]+$)")


IMAGE = re.compile(r"\.(png|jpe?g|webp)$", re.I)
# Field names that can hold a URLs-table code instead of a URL (SSGF's tables refer to URLs by code).
# Not icon fields: their small item-type numbers collide with URL codes.
URL_CODE_FIELD = re.compile(r"image|banner|bg|url|thumb|album|emblem|frame|preview|gif|resource", re.I)
URL_LISTS = {"URLs", "AssetURLData"}  # list every asset; a mention there doesn't mean anything shows it


def hidden_images(loaded):
    """Images that only tables behind hidden menus use (game.json "hiddenTables": events, missions,
    passes, removed shop pages). The server answers missing images with a placeholder, so if one
    is still needed somewhere the worst case is a blank picture. Kept: anything another table
    (other than the URL lists) refers to, by URL or by URLs-table code."""
    hidden = set(game.cfg.get("hiddenTables", []))
    if not hidden:
        return set()
    by_code = {r["code"]: r.get("url") or "" for r in loaded.get("URLs", []) if isinstance(r, dict) and "code" in r}

    def table_refs(rows):
        out = {
            str(tables.rel_path(u))
            for u in URL_RE.findall(json.dumps(rows, ensure_ascii=False).replace("\\/", "/"))
            if u.startswith("http")
        }
        for row in rows if isinstance(rows, list) else []:
            for k, v in row.items() if isinstance(row, dict) else []:
                if URL_CODE_FIELD.search(k):
                    for x in v if isinstance(v, list) else [v]:
                        if isinstance(x, int) and by_code.get(x, "").startswith("http"):
                            out.add(str(tables.rel_path(by_code[x])))
        return out

    used_hidden, used_shown = set(), set()
    for name, rows in loaded.items():
        if name in URL_LISTS:
            continue
        (used_hidden if name in hidden else used_shown).update(table_refs(rows))
    # Language variants count as the same picture: keep all of them if a shown table uses one.
    shown_bases = {LOCALE_SUFFIX.sub("", p) for p in used_shown}
    return {p for p in used_hidden - used_shown if IMAGE.search(p) and LOCALE_SUFFIX.sub("", p) not in shown_bases}


def write_list():
    """Every file in cdn-mirror/ minus files that only removed content references."""
    dm = tables.load_decrypt()
    rules = tables.load_rules()
    ctx = json.load(open(ASSET_CONTEXT, encoding="utf-8-sig"))["context"]

    def refs(rows):
        text = json.dumps(rows, ensure_ascii=False).replace("\\/", "/")
        return {str(tables.rel_path(u)) for u in URL_RE.findall(text) if u.startswith("http")}

    loaded = {}
    for name, entry in ctx.items():
        url = entry.get("file") if isinstance(entry, dict) else None
        if not url or url.endswith("/") or not (MIRROR / tables.rel_path(url)).exists():
            continue
        try:
            loaded[name] = json.loads(dm.decrypt((MIRROR / tables.rel_path(url)).read_bytes()))
        except Exception:
            continue
    removed_tabs = set()
    for rule in rules.get("StoreMenuData", []):
        if rule.get("remove"):
            removed_tabs |= {r["code"] for r in loaded.get("StoreMenuData", []) if tables.matches(r, rule["match"])}

    before, after = set(), set()
    for name, rows in loaded.items():
        if not isinstance(rows, list):
            before |= refs(rows)
            after |= refs(rows)
            continue
        before |= refs(rows)
        kept, _, _ = tables.apply_rules([dict(r) for r in rows], rules.get(name, []))
        kept = [r for r in kept if not (isinstance(r, dict) and r.get("storeMenuID") in removed_tabs)]
        after |= refs(kept)
    gone = before - after
    gone_bases = {LOCALE_SUFFIX.sub("", p) for p in gone}
    hidden = hidden_images(loaded)

    # Tables: original URLs, and the folders they live in (other files there are old tables).
    table_urls = {
        str(tables.rel_path(e["file"])): e["file"]
        for e in ctx.values()
        if isinstance(e, dict) and e.get("file") and not e["file"].endswith("/")
    }
    table_dirs = {f.rpartition("/")[0] for f in table_urls}

    files = sorted(
        str(p.relative_to(MIRROR))
        for p in MIRROR.rglob("*")
        if p.is_file() and p.name not in ("CACHEDIR.TAG", ".gitkeep")
    )
    old_tables = [f for f in files if f.rpartition("/")[0] in table_dirs and f not in table_urls]
    keep = [
        f
        for f in files
        if f not in gone and LOCALE_SUFFIX.sub("", f) not in gone_bases and f not in old_tables and f not in hidden
    ]
    sources = game.cfg.get("cdnSources", {})
    with open(LIST, "w") as out:
        out.write(
            "# Every CDN file the offline build needs, as aria2c input (written by\n"
            f"# tools/make-aria2.py). Download into cdn-mirror/{game.id}/ from the project root:\n"
            f"#   aria2c -i games/{game.id}/cdn-files.aria2 -c -j 8 --auto-file-renaming=false\n"
        )
        for f in keep:
            host, path = f.split("/", 1)
            d, _, name = f.rpartition("/")
            url = (
                table_urls.get(f, "").replace("http://", "https://", 1)
                or f"https://{sources.get(host, host)}/{quote(path, safe=PATH_SAFE)}"
            )
            out.write(f"{url}\n  dir=cdn-mirror/{game.id}/{d}\n  out={name}\n")
    print(
        f"{len(files)} files in cdn-mirror/: {len(old_tables)} tables of other index versions,"
        f" {len(files) - len(keep) - len(old_tables)} only used by removed content;"
        f" wrote {len(keep)} to {LIST.relative_to(ROOT)}"
    )


if __name__ == "__main__":
    write_list()
