"""The game's AES-256 key (master data tables and API bodies), read from your own copy of the game.

The key is a 32-character string in libssnative.so. This repository only keeps its SHA-256, so the key itself
isn't published: game_key() checks every 32-character run of printable bytes in the library against the hash.
The hash is "keySha256" in games/<game>/game.json. Needs work/<game>/unpacked/ (tools/unpack.sh).
"""

import hashlib, re, sys

from game import ROOT, current


def game_key() -> bytes:
    game = current()
    library = game.unpacked / "arm64/lib/arm64-v8a/libssnative.so"
    if not library.exists():
        sys.exit(f"{library.relative_to(ROOT)} not found; unpack the game first (tools/unpack.sh {game.id})")
    data = library.read_bytes()
    for m in re.finditer(rb"(?=([\x21-\x7e]{32}))", data):
        if hashlib.sha256(m.group(1)).hexdigest() == game["keySha256"]:
            return m.group(1)
    sys.exit(f"the game key wasn't found in {library.relative_to(ROOT)}; is this version {game['version']}?")
