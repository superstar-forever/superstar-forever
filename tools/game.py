"""Which game a tool works on, and where that game's files are.

Every tool takes `--game <id>` (or the GAME environment variable); <id> is a folder in games/.

  games/<id>/            in git: game.json, patches.json, table overrides and versions,
                         progression.json, manifest.json, api-overrides.json, cdn-files.aria2,
                         re/ (notes, API catalog, captures such as the table index)
  original/              not in git: the original .xapk/.apk files, any name (tools/unpack.sh)
  cdn-mirror/<id>/       not in git: downloaded game files, <host>/<path> like the original URLs
  build/<id>/            not in git: patched files, APKs, the .apks (all recreated by the tools)
  keys/<id>.keystore     not in git: the signing key (created by tools/build-apk.py; back it up)
  work/<id>/             not in git, recreated by the tools:
    unpacked/            tools/unpack.sh
    server-data/         tools/build-server-data.py
    extracted/           tools/decrypt-masterdata.py
    re/                  IL2CPP dump, call index and other generated reverse-engineering files

Usage in a tool:
    from game import select
    game, argv = select()          # parses and removes --game from sys.argv
    game.work / "unpacked"         # or the shortcuts below
Shared modules (gamekey.py, tables.py) use current(), the game selected for this run.
"""

import json, os, sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
GAMES = ROOT / "games"


class Game:
    def __init__(self, gid: str):
        self.id = gid
        self.dir = GAMES / gid
        self.work = ROOT / "work" / gid
        self.cfg = json.load(open(self.dir / "game.json", encoding="utf-8"))

    def __getitem__(self, key):
        return self.cfg[key]

    # games/<id>/ (committed)
    def file(self, name: str) -> Path:
        return self.dir / name

    @property
    def re(self) -> Path:
        return self.dir / "re"

    @property
    def asset_context(self) -> Path:
        return self.dir / self.cfg["assetContext"]

    # local, not in git
    @property
    def original(self) -> Path:
        return ROOT / "original"

    @property
    def unpacked(self) -> Path:
        return self.work / "unpacked"

    @property
    def mirror(self) -> Path:
        return ROOT / "cdn-mirror" / self.id

    @property
    def server_data(self) -> Path:
        return self.work / "server-data"

    @property
    def extracted(self) -> Path:
        return self.work / "extracted"

    @property
    def build(self) -> Path:
        return ROOT / "build" / self.id

    @property
    def re_local(self) -> Path:
        return self.work / "re"

    @property
    def libil2cpp(self) -> Path:
        return self.unpacked / "arm64/lib/arm64-v8a/libil2cpp.so"

    @property
    def keystore(self) -> Path:
        return ROOT / "keys" / f"{self.id}.keystore"


_current = None


def current() -> "Game":
    """The game this run works on (selected from --game / $GAME on first use)."""
    return _current or select()[0]


def available() -> list:
    return sorted(p.name for p in GAMES.iterdir() if (p / "game.json").exists())


def select(argv=None):
    """Returns (Game, remaining argv). Takes --game ID / --game=ID, else $GAME; exits otherwise."""
    argv = list(sys.argv if argv is None else argv)
    gid = None
    for i, a in enumerate(argv):
        if a == "--game" and i + 1 < len(argv):
            gid = argv[i + 1]
            del argv[i : i + 2]
            break
        if a.startswith("--game="):
            gid = a.split("=", 1)[1]
            del argv[i]
            break
    gid = gid or os.environ.get("GAME")
    if not gid or gid not in available():
        sys.exit(f"{Path(argv[0]).name}: pass --game <id> (one of: {', '.join(available())})")
    global _current
    sys.argv[:] = argv
    _current = Game(gid)
    return _current, argv


if __name__ == "__main__":
    # For shell scripts: `eval "$(tools/game.py --shell <id>)"` sets GAME, GAME_DIR, WORK, BUILD, PACKAGE.
    if sys.argv[1:2] == ["--shell"] and len(sys.argv) > 2:
        g = Game(sys.argv[2]) if sys.argv[2] in available() else sys.exit(f"unknown game {sys.argv[2]}")
        print(f"GAME={g.id}; GAME_DIR='{g.dir}'; WORK='{g.work}'; BUILD='{g.build}'; PACKAGE='{g['package']}'")
    else:
        print("\n".join(available()))
