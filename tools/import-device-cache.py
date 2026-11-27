#!/usr/bin/env python3
"""Rebuild cdn-mirror/<game>/<host>/<path> from the game's on-device cache (when the CDN is gone).

Usage: tools/import-device-cache.py --game <id> <cache dir> [--copy] [--dry-run]

<cache dir> is a copy of the game's cache folder (the one with the download tree; on the phone
under Android/data/<package>/files/). The download tree is found by its table index (a.json).
Nothing in <cache dir> is changed: files are hardlinked into cdn-mirror/<game>/ (--copy copies).
Existing mirror files are kept; a different file already at a path is reported, not replaced.

The game stores downloads under obfuscated names, so the URL of each cached file comes from
(games/<game>/device-cache.json says where each of these is):
  1. tables: the cached .bin of each table of the asset context goes to the table's URL there
     (the cached files are byte-identical to the CDN/S3 ones)
  2. the game's own URL -> cache path map ("urlMap", e.g. url_shared_map.bin)
  3. files stored by a table row's code ("byCode", e.g. songs and charts: <code>.a.bytes)
  4. files the map doesn't list, by the patterns the map shows: if the map puts table T's field F
     (row code C, optional language suffix L) at "<folder>/<C>_xx.i.bytes", every other row of T
     is looked for at the same pattern
  5. what's still left: version (urlVersion: cache path -> v=) and size equal to exactly one URL
     in the size index (urlSizes: URL?v= -> size)
  6. title backgrounds: "titleBackgrounds" (cache path with {0}) -> the manifest's
     TitleBackgroundUrl for 0 .. TitleBackgroundCount - 1
File names may carry an extra extension added when copying the cache off the phone
(1001.a.bytes.ogg for 1001.a.bytes); both forms are accepted.

Report: what was placed by which source, cache files that couldn't be placed (by folder),
and URLs the tables refer to that the mirror still lacks (by table and field). The full lists go
to work/<game>/device-cache-import.json.
"""

import json, os, re, shutil, sys
from collections import Counter, defaultdict
from pathlib import Path
from urllib.parse import urlsplit

sys.path.insert(0, str(Path(__file__).resolve().parent))
from game import ROOT, available, select
import tables

game, argv = select()
args = [a for a in argv[1:] if not a.startswith("--")]
COPY = "--copy" in argv
DRY = "--dry-run" in argv
if len(args) != 1:
    sys.exit(__doc__)
CACHE = Path(args[0])
CFG = json.load(open(game.file("device-cache.json"), encoding="utf-8"))
MIRROR = game.mirror

# Language suffix the game adds to a base name at runtime (shop_x.png -> shop_x_enUS.png, ..._en.png).
LOCALE = re.compile(r"_([a-z]{2}[A-Z]{2}|[a-z]{2})(?=\.[^./]+$)")
dm = tables.load_decrypt()


def strip_copy_ext(name):
    """1001.a.bytes.ogg -> 1001.a.bytes (the game writes <name>.a.bytes / .i.bytes)."""
    return re.sub(r"(\.bytes)\.[A-Za-z0-9]+$", r"\1", name)


def find_tree():
    for p in sorted(CACHE.rglob("a.json")):
        if p.parent.name == CFG["dataFolder"]:
            return p.parent.parent
    sys.exit(f"no {CFG['dataFolder']}/a.json under {CACHE}")


TREE = find_tree()
DATA = TREE / CFG["dataFolder"]

files = {}  # cache path (relative to TREE, copy extension removed) -> real file
for p in TREE.rglob("*"):
    if not p.is_file() or p.name.endswith(".ver") or any(s.startswith(".") for s in p.relative_to(TREE).parts):
        continue
    rel = p.relative_to(TREE)
    files[str(rel.parent / strip_copy_ext(rel.name))] = p


def load(name):
    return json.loads(dm.decrypt((DATA / name).read_bytes())) if name and (DATA / name).is_file() else None


stats = Counter()
placed = {}  # cache path -> set of mirror paths
by_rel = {}  # mirror path -> cache path
conflicts = []  # (cache path, mirror path): another file is already there
duplicates = {}  # cache path -> cache path with the same content placed at the same URL


def same(a: Path, b: Path):
    return a.samefile(b) or a.stat().st_size == b.stat().st_size and a.read_bytes() == b.read_bytes()


def link(src: Path, dst: Path):
    if DRY:
        return
    dst.parent.mkdir(parents=True, exist_ok=True)
    if COPY:
        shutil.copy2(src, dst)
    else:
        try:
            os.link(src, dst)
        except OSError:
            shutil.copy2(src, dst)


left_out = set()  # mirror paths only hidden or emptied tables use (filled after the tables are read)


def place(cpath, url, source):
    rel = tables.rel_path(url)
    key = str(rel)
    if key in left_out or LOCALE.sub("", key) in left_out:  # language variants count as their base file
        stats["left out (only used by hidden or emptied tables)"] += 1
        placed.setdefault(cpath, set())
        return
    src = files[cpath]
    if key in by_rel:
        other = by_rel[key]
        if other != cpath and cpath not in placed and cpath not in duplicates:
            if same(src, files[other]):
                duplicates[cpath] = other
            else:
                conflicts.append((cpath, key))
        return
    dst = MIRROR / rel
    if dst.exists():
        if not same(dst, src):
            conflicts.append((cpath, key))
            return
        stats[f"{source} (already in mirror)"] += 1
    else:
        stats[source] += 1
        link(src, dst)
    by_rel[key] = cpath
    placed.setdefault(cpath, set()).add(key)


def url_fields(rows):
    for row in rows if isinstance(rows, list) else []:
        if isinstance(row, dict):
            for k, v in row.items():
                if isinstance(v, str) and v.startswith("http"):
                    yield row, k, v


def is_file_url(url):
    """A field that is one URL of a file (not text with a link, not a folder or web page)."""
    return not re.search(r"\s", url) and re.search(r"/[^/]+\.\w{2,5}$", urlsplit(url).path) is not None


def norm(url):
    return str(tables.rel_path(url))


def has_url(url):
    return str(tables.rel_path(url)) in by_rel or (MIRROR / tables.rel_path(url)).is_file()


# 1. tables of the asset context
ctx = json.load(open(game.asset_context, encoding="utf-8-sig"))["context"]
cached_ctx = DATA / "a.json"
if cached_ctx.is_file():
    v_cache = json.load(open(cached_ctx, encoding="utf-8-sig")).get("version")
    v_game = json.load(open(game.asset_context, encoding="utf-8-sig")).get("version")
    if v_cache != v_game:
        print(f"note: the cache's a.json is version {v_cache}, {game.asset_context.name} is {v_game}")
table_rows = {}
tables_missing = []
for name, entry in sorted(ctx.items()):
    url = entry.get("file") if isinstance(entry, dict) else None
    if not url or url.endswith("/"):
        continue
    fname = CFG["tables"].get(name)
    cpath = f"{CFG['dataFolder']}/{fname}" if fname else None
    if cpath in files:
        place(cpath, url, "tables")
        rows = load(fname)
    else:
        src = MIRROR / tables.rel_path(url)
        tables_missing.append((name, url, src.is_file()))
        rows = json.loads(dm.decrypt(src.read_bytes())) if src.is_file() else None
    if rows is not None:
        table_rows[name] = rows

# Tables the offline game doesn't show (game.json "hiddenTables", tables table-overrides.json empties):
# files only they use are left out of the mirror (e.g. SSLO's live theme, a leftover copy of SSOMG's).
overrides = (
    json.load(open(game.file("table-overrides.json"), encoding="utf-8"))
    if game.file("table-overrides.json").exists()
    else {}
)
emptied = {
    t
    for t, rules in overrides.items()
    if isinstance(rules, list) and any(r.get("match") == {} and r.get("remove") for r in rules)
}
SKIP_TABLES = {"URLs", "AssetURLData"} | set(game.cfg.get("hiddenTables", [])) | emptied
kept_urls, skipped_urls = set(), set()
for name, rows in table_rows.items():
    if name in ("URLs", "AssetURLData"):
        continue
    for row, field, url in url_fields(rows):
        if is_file_url(url):
            (skipped_urls if name in SKIP_TABLES else kept_urls).add(norm(url))
left_out.update(skipped_urls - kept_urls)

# 2. the game's URL map
url_map = load(CFG.get("urlMap")) or {}
for url, cpath in url_map.items():
    if cpath in files:
        place(cpath, url, "URL map")

versions = load(CFG.get("urlVersion")) or {}
sizes = load(CFG.get("urlSizes")) or {}


def size_ok(cpath, url):
    """False if the size index knows this URL at the cached file's version with another size."""
    v = versions.get(cpath + ".ver")
    s = sizes.get(f"{url.partition('?')[0]}?{v}") if v else None
    return s is None or s == files[cpath].stat().st_size


# 3. files stored by code
for t in CFG.get("byCode", []):
    for row in table_rows.get(t["table"], []):
        url = row.get(t["field"])
        cpath = t["path"].format(**row)
        if isinstance(url, str) and url.startswith("http") and cpath in files and size_ok(cpath, url):
            place(cpath, url, "by code")


# 4. patterns learnt from the URL map
base_index = defaultdict(list)  # mirror path of a table URL -> (table, field, row)
for name, rows in table_rows.items():
    if name in ("URLs", "AssetURLData"):  # URL lists, not what a picture belongs to
        continue
    for row, k, v in url_fields(rows):
        base_index[norm(v)].append((name, k, row))

votes = defaultdict(Counter)  # (table, field, locale) -> Counter of (key field, path template)
entries = Counter()  # (table, field, locale) -> map entries seen
for url, cpath in url_map.items():
    rel = norm(url)
    candidates = [(rel, None)]
    m = LOCALE.search(rel)
    if m:
        candidates.append((rel[: m.start()] + rel[m.end() :], m.group(1)))
    for base, loc in candidates:
        for name, field, row in base_index.get(base, []):
            entries[(name, field, loc)] += 1
            for kf, kv in row.items():
                if isinstance(kv, int) and not isinstance(kv, bool):
                    tpl, n = re.subn(rf"(?<!\d){kv}(?!\d)", "{key}", cpath, count=1)
                    if n:
                        votes[(name, field, loc)][(kf, tpl)] += 1
# A pattern is trusted when most of the map entries of that table field follow it.
patterns = {}
for key, c in votes.items():
    (kf, tpl), n = c.most_common(1)[0]
    if n >= 2 and n >= 0.8 * entries[key]:
        patterns[key] = (kf, tpl)
for (name, field, loc), (kf, tpl) in patterns.items():
    for row in table_rows.get(name, []):
        url = row.get(field) if isinstance(row, dict) else None
        if not isinstance(url, str) or not url.startswith("http") or not isinstance(row.get(kf), int):
            continue
        cpath = tpl.replace("{key}", str(row[kf]))
        if cpath not in files:
            continue
        if loc:
            base, _, query = url.partition("?")
            stem, dot, ext = base.rpartition(".")
            url = f"{stem}_{loc}.{ext}" + (f"?{query}" if query else "")
        if size_ok(cpath, url):
            place(cpath, url, "map patterns")
        else:
            stats["map patterns: size mismatch, skipped"] += 1

# 5. version + size
by_vs = defaultdict(set)
for url, size in sizes.items():
    by_vs[(url.partition("?")[2], size)].add(url.partition("?")[0])
for cpath, src in sorted(files.items()):
    if cpath in placed or cpath in duplicates or cpath.startswith(CFG["dataFolder"] + "/"):
        continue
    v = versions.get(cpath + ".ver")
    cands = by_vs.get((v, src.stat().st_size), set()) if v else set()
    if len(cands) > 1:  # several URLs fit: the ones not taken yet
        cands = {u for u in cands if str(tables.rel_path(u)) not in by_rel}
        cands = {
            u for u in cands if not (MIRROR / tables.rel_path(u)).exists() or same(MIRROR / tables.rel_path(u), src)
        }
    if len(cands) == 1:
        place(cpath, cands.pop(), "version + size")

# 6. title backgrounds
manifest = json.load(open(game.file("manifest.json"), encoding="utf-8"))
title_missing = []
if CFG.get("titleBackgrounds") and manifest.get("TitleBackgroundUrl"):
    for i in range(int(manifest.get("TitleBackgroundCount") or 0)):
        cpath = CFG["titleBackgrounds"].format(i)
        url = manifest["TitleBackgroundUrl"].replace("{0}", str(i))
        if cpath in files:
            place(cpath, url, "title backgrounds")
        elif not has_url(url):
            title_missing.append(url)

# 7. files on hosts several games share (Dalcomsoft's common CDNs, another game's CDN): the same
# URL path in another game's mirror is the same file
needed = defaultdict(set)  # "Table.field" -> mirror paths of the file URLs in it
for name, rows in table_rows.items():
    if name in SKIP_TABLES:
        continue
    for row, field, url in url_fields(rows):
        if is_file_url(url):
            needed[f"{name}.{field}"].add(norm(url))
shared = Counter()
for rel in sorted(set().union(*needed.values()) if needed else ()):
    if rel in by_rel or (MIRROR / rel).exists():
        continue
    for other in available():
        src = ROOT / "cdn-mirror" / other / rel
        if other != game.id and src.is_file():
            link(src, MIRROR / rel)
            by_rel[rel] = f"cdn-mirror/{other}/{rel}"
            shared[other] += 1
            break

# ------------------------------------------------------------------ report
print(f"cache: {len(files)} files in {TREE}")
for k, n in sorted(stats.items()):
    print(f"  {n:6d}  {k}")
for other, n in sorted(shared.items()):
    print(f"  {n:6d}  from cdn-mirror/{other}/ (same URL on a host the games share)")
print(f"  {len(by_rel)} mirror paths, {len(placed)} cache files used" + (" (dry run)" if DRY else ""))
if duplicates:
    print(f"  {len(duplicates)} more cache files are copies of placed ones (same URL, same content)")
if conflicts:
    print(f"  {len(conflicts)} not placed: a different file is already at that URL (e.g. {conflicts[0][1]})")

unplaced = sorted(c for c in files if c not in placed and c not in duplicates)
by_folder = Counter(c.rpartition("/")[0] for c in unplaced)
print(f"\ncache files not placed: {len(unplaced)}")
for f, n in by_folder.most_common():
    print(f"  {n:6d}  {f}/")

if tables_missing:
    print("\ntables the cache doesn't have:")
    for name, url, have in tables_missing:
        print(f"  {name}: {'in the mirror already' if have else 'MISSING'} ({url})")
if title_missing:
    print(f"\ntitle backgrounds missing: {len(title_missing)}")

# URLs the tables refer to that the mirror lacks. A base name counts as present when the game's
# runtime variants are (language _enUS / _en, size _Large, ...: same folder, name + "_..."
# and same extension).
present = set(by_rel)
for p in MIRROR.rglob("*"):
    if p.is_file():
        present.add(str(p.relative_to(MIRROR)))
variant_stems = set()
for p in present:
    d, _, n = p.rpartition("/")
    stem, _, ext = n.rpartition(".")
    for i in [m.start() for m in re.finditer("_", stem)]:
        variant_stems.add(f"{d}/{stem[:i]}.{ext}")
missing = defaultdict(set)
for k, rels in needed.items():
    for rel in rels:
        if rel not in present and rel not in variant_stems:
            missing[k].add(rel)
total_needed = len(set().union(*needed.values())) if needed else 0
total_missing = set().union(*missing.values()) if missing else set()
print(f"\nURLs in the tables: {total_needed}, not in the mirror: {len(total_missing)}")
kinds = Counter(re.sub(r".*\.", ".", r.lower()) for r in total_missing)
print("  by type: " + ", ".join(f"{k} {n}" for k, n in kinds.most_common()))
for k in sorted(missing, key=lambda k: -len(missing[k])):
    print(f"  {len(missing[k]):5d} / {len(needed[k]):5d}  {k}")

report = {
    "placed": {k: sorted(v) for k, v in sorted(placed.items())},
    "unplaced": unplaced,
    "duplicates": duplicates,
    "conflicts": conflicts,
    "tablesMissing": [{"table": n, "url": u, "inMirror": h} for n, u, h in tables_missing],
    "titleBackgroundsMissing": title_missing,
    "missingByField": {k: sorted(v) for k, v in sorted(missing.items())},
    "patterns": {
        f"{t}.{f}" + (f" [{loc}]" if loc else ""): tpl for (t, f, loc), (_, tpl) in sorted(patterns.items(), key=str)
    },
}
if not DRY:
    out = game.work / "device-cache-import.json"
    out.parent.mkdir(parents=True, exist_ok=True)
    out.write_text(json.dumps(report, indent=1, ensure_ascii=False) + "\n", encoding="utf-8")
    print(f"\nfull lists: {out.relative_to(ROOT)}")
