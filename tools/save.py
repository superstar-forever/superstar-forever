#!/usr/bin/env python3
"""Back up, edit or restore the player's save on a phone, through the server inside the game.

Usage:
  tools/save.py --game <id> pull [FILE]           save -> FILE (default: <game>.save.json)
  tools/save.py --game <id> push FILE             FILE -> save, then restart the game
  tools/save.py --game <id> boost [RP] [DIAMONDS] add RP and diamonds (default 1,000,000 / 10,000)

The game has to be running (it hosts the server). Uses `adb forward tcp:<port>` (game.json "port"); pass
-s SERIAL first to pick a device, like with adb. The save is plain JSON: balances are in
"player" (freeRhythmPoint, freeDiamond), cards in "cards" (cardID, grade, level, objectID).
"""

import json, subprocess, sys, urllib.request
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from game import select

GAME, _ = select()
PACKAGE = GAME["package"]
PORT = GAME.cfg.get("port", 18384)


def adb(serial, *args):
    cmd = ["adb"] + (["-s", serial] if serial else []) + list(args)
    return subprocess.run(cmd, check=True, capture_output=True, text=True).stdout


def url(path):
    return f"http://127.0.0.1:{PORT}{path}"


def pull():
    with urllib.request.urlopen(url("/save/full"), timeout=10) as r:
        return json.load(r)


def push(serial, save):
    req = urllib.request.Request(url("/save/full"), data=json.dumps(save).encode(), method="PUT")
    try:
        with urllib.request.urlopen(req, timeout=10):
            pass
    except urllib.error.HTTPError as e:
        sys.exit(f"rejected: {e.read().decode().strip()}")
    # The game keeps its own copy of the player until it logs in again.
    adb(serial, "shell", "am", "force-stop", PACKAGE)
    adb(serial, "shell", "monkey", "-p", PACKAGE, "-c", "android.intent.category.LAUNCHER", "1")
    print("save replaced, game restarted")


def main():
    args = sys.argv[1:]
    serial = None
    if args[:1] == ["-s"] and len(args) >= 2:
        serial, args = args[1], args[2:]
    if not args or args[0] not in ("pull", "push", "boost"):
        sys.exit(__doc__)
    adb(serial, "forward", f"tcp:{PORT}", f"tcp:{PORT}")
    try:
        save = pull()
    except OSError as e:
        sys.exit(f"can't reach the server ({e}); is the game running?")

    if args[0] == "pull":
        path = args[1] if len(args) > 1 else f"{GAME.id}.save.json"
        with open(path, "w") as f:
            json.dump(save, f, indent=2, ensure_ascii=False)
        print(f"wrote {path} ({len(save.get('cards', []))} cards)")
    elif args[0] == "push":
        if len(args) < 2:
            sys.exit("push needs a file")
        with open(args[1]) as f:
            push(serial, json.load(f))
    else:
        rp = int(args[1]) if len(args) > 1 else 1_000_000
        dia = int(args[2]) if len(args) > 2 else 10_000
        p = save["player"]
        p["freeRhythmPoint"] = p.get("freeRhythmPoint", 0) + rp
        p["freeDiamond"] = p.get("freeDiamond", 0) + dia
        push(serial, save)
        print(f"+{rp:,} RP, +{dia:,} diamonds")


if __name__ == "__main__":
    main()
