#!/usr/bin/env python3
"""Apply the byte patches in games/<game>/patches.json to a copy of the target in build/<game>/.

Usage: tools/apply-patches.py --game <id> [patches.json]
Each patch checks the original bytes first, so a different game version fails loudly
instead of being corrupted.
"""

import json, shutil, sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from game import ROOT, select

game, argv = select()
spec_path = Path(argv[1]) if len(argv) > 1 else game.file("patches.json")
spec = json.load(open(spec_path))
src = game.work / spec["target"]  # relative to work/<game>/
dst = game.build / src.name
dst.parent.mkdir(parents=True, exist_ok=True)
shutil.copyfile(src, dst)

with open(dst, "r+b") as f:
    for p in spec["patches"]:
        off = int(p["file_offset"], 16)
        orig, new = bytes.fromhex(p["original"]), bytes.fromhex(p["patched"])
        f.seek(off)
        cur = f.read(len(orig))
        if cur == new:
            print(f"= {p['name']}: already patched")
            continue
        if cur != orig:
            sys.exit(f"! {p['name']}: expected {orig.hex()} at {off:#x}, found {cur.hex()}")
        f.seek(off)
        f.write(new)
        print(f"+ {p['name']}: {orig.hex()} -> {new.hex()} at {off:#x}")
print(f"Wrote {dst.relative_to(ROOT)}")
