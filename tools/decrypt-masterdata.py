#!/usr/bin/env python3
"""Decrypt the game's master data tables into readable JSON.

Usage: tools/decrypt-masterdata.py --game <id> [output-dir]
  every table in the game's asset context (games/<game>/game.json "assetContext", the index the
  game downloads) from cdn-mirror/<game>/  ->  work/<game>/extracted/masterdata/<Table>.json

File format (from CipherSimple in libil2cpp.so):
  optional gzip  ->  base64  ->  AES-256-ECB, PKCS7 padding  ->  UTF-8 JSON
The key is a string literal used by CipherSimple.open() (read from libssnative.so, see tools/gamekey.py).
"""

import base64, gzip, json, sys
from pathlib import Path

from Crypto.Cipher import AES
from Crypto.Util.Padding import unpad

sys.path.insert(0, str(Path(__file__).resolve().parent))
from game import current
from gamekey import game_key

KEY = game_key()  # from your libssnative.so, see tools/gamekey.py


def decrypt(raw: bytes) -> bytes:
    if raw[:2] == b"\x1f\x8b":
        raw = gzip.decompress(raw)
    data = AES.new(KEY, AES.MODE_ECB).decrypt(base64.b64decode(raw))
    return unpad(data, 16)


def encrypt(plain: bytes, compress: bool = True) -> bytes:
    """Inverse of decrypt(), for writing tables the game can load."""
    from Crypto.Util.Padding import pad

    data = base64.b64encode(AES.new(KEY, AES.MODE_ECB).encrypt(pad(plain, 16)))
    return gzip.compress(data) if compress else data


def main():
    import tables

    game = current()
    out = Path(sys.argv[1]) if len(sys.argv) > 1 else game.extracted / "masterdata"
    out.mkdir(parents=True, exist_ok=True)
    ctx = json.load(open(game.asset_context, encoding="utf-8-sig"))["context"]
    count = 0
    for name, entry in sorted(ctx.items()):
        url = entry.get("file") if isinstance(entry, dict) else None
        if not url or url.endswith("/"):
            continue
        src = tables.mirror() / tables.rel_path(url)
        try:
            obj = json.loads(decrypt(src.read_bytes()))
        except Exception as e:
            print(f"! {name}: {e}")
            continue
        (out / f"{name}.json").write_text(json.dumps(obj, ensure_ascii=False, indent=1), encoding="utf-8")
        count += 1
    print(f"{count} tables written to {out}")


if __name__ == "__main__":
    main()
