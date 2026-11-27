"""Shared helpers for the game's master data tables (used by build-server-data.py and
make-aria2.py).

- load_decrypt(): tools/decrypt-masterdata.py as a module (decrypt/encrypt).
- rel_path(): where a file from a CDN URL lives in cdn-mirror/<game>/.
- apply_rules(): games/<game>/table-overrides.json rules for one table.
"""

import importlib.util, json, re
from pathlib import Path
from urllib.parse import urlsplit

from game import ROOT, current


def mirror() -> Path:
    return current().mirror


def load_decrypt():
    spec = importlib.util.spec_from_file_location("dm", ROOT / "tools/decrypt-masterdata.py")
    dm = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(dm)
    return dm


def rel_path(url):
    """<host>/<path> of a CDN URL as laid out in cdn-mirror/ (query dropped, // collapsed)."""
    u = urlsplit(url)
    return Path(u.netloc) / re.sub("/+", "/", u.path).lstrip("/")


def load_rules():
    rules = json.load(open(current().file("table-overrides.json"), encoding="utf-8"))
    return {k: v for k, v in rules.items() if not k.startswith("_")}


def matches(row, match):
    # A list in "match" means "any of these values".
    return all(row.get(k) in v if isinstance(v, list) else row.get(k) == v for k, v in match.items())


def apply_rules(rows, table_rules):
    """Applies one table's rules. Returns (rows, changed count, removed count)."""
    changed = removed = 0
    for rule in table_rules:
        if rule.get("remove"):
            before = len(rows)
            rows = [r for r in rows if not matches(r, rule["match"])]
            removed += before - len(rows)
            continue
        for row in rows:
            if matches(row, rule["match"]):
                row.update(rule["set"])
                changed += 1
    return rows, changed, removed
