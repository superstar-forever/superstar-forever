# Master data

## Format

The table files (on the CDN as listed in the asset context, on the phone in `asiudgasg/sdisgkdc/`):
optional gzip → base64 → **AES-256-ECB**, PKCS7 → UTF-8 JSON.

- Implemented in `com.dalcomsoft.project.client.CipherSimple`. `open()` @ `0x1EB2408` creates
  `Rijndael` with Mode = ECB, Padding = PKCS7, BlockSize = 128, KeySize = 256, and
  Key = UTF-8 of a 32-character literal from libssnative.so (`tools/gamekey.py` finds it by its SHA-256).
- No IV (ECB). The same key is used by all three games.
- `tools/decrypt-masterdata.py` decrypts every table of the asset context from `cdn-mirror/<game>/` to
  `work/<game>/extracted/masterdata/<Table>.json`. It also has `encrypt()` for writing tables back.

## File names → tables

- Set in the static constructor of `com.dalcomsoft.project.app.control.AssetPathControl`
  (`PATH_FILE_ASSET_*` fields), e.g. `msd.bin` = MUSIC, `cdd.bin` = CARD, `atd.bin` = ARTIST.
- The full list is in `tools/decrypt-masterdata.py` (`TABLES`).
- Heads-up when reading the disassembly: each literal is stored to the field Il2CppDumper
  lists 8 bytes *after* the offset in the `str` instruction.
- `tools/literals.py --game <id> <start> <end>` disassembles with string literals and types resolved.
- Table names are those of the asset context (e.g. `StoreMenuData`).
- Tables the client knows but the asset context doesn't list (e.g. `MAJOR_GROUP`, `HIDDEN_GAME`, `BUFF`, the
  `MUSICBATTLE_*` tables) are never downloaded; the game copes without them.

## Asset cache index

The device cache keeps these files next to the tables:

| File | Content |
|---|---|
| `url_shared_map.json` | CDN URL → local cache path (e.g. `asodhsg/bhfbfhbf/201.i.bytes`) |
| `url_version.json` | Local cache path `.ver` → version query (`v=...`) |
| `url_v2.json` | CDN URL → file size |
| `a.json` (plain JSON) | Table name → S3 URL + version (the asset context) |

## Songs

MusicData has a `code`, `sound`, and `seqEasy` / `seqNormal` / `seqHard` / `seqMusicBattle` URLs per song.
The `_4`, `_7` and `_13` suffixes on the charts are the difficulty.

Audio and charts are cached by code, not via `url_shared_map`:

| Path | Content |
|---|---|
| `dslkjfsdalhh/<code>.a.bytes` | Full song (OGG) |
| `dslkjfsdalhh/asdlkasdqoweirypqwier/<code>.a.bytes` | 12-second preview |
| `dslkjfsdalhh/hidhhz/*.a.bytes` | Artist pack voice previews |
| `aweid/<code>_<4\|7\|13>.seq` | Note charts |
