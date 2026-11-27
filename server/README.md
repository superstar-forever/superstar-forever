# superstar-server

Offline replacement for the game server of Dalcomsoft's SuperStar games (so far SUPERSTAR OH MY GIRL), written in Rust.
It serves the startup manifest, all game files (as a local CDN) and the game's RPC API, for one fixed player, "Miracle".

## Build and run (desktop)

```bash
# 1. Assemble work/ssomg/server-data/ (hardlinks, no extra disk space). Re-run after new downloads.
tools/build-server-data.py --game ssomg

# 2. Build.
cd server && cargo build --release

# 3. Run. --public-url must be how the game reaches this machine.
server/target/release/superstar-server --data work/ssomg/server-data --bind 0.0.0.0:18384 \
    --public-url http://<this-machine-ip>:18384 --save ssomg.save.json --log server.log
# Only CDN misses are logged; SUPERSTAR_LOG_CDN=1 logs every file served.

# 4. Smoke test (startup, master data, login, tutorial, songs, progression)
tools/test-client.py --game ssomg http://127.0.0.1:18384
```

## How it works

| Route | Answer |
|---|---|
| `GET /health` | `ok` |
| `GET /save` | The player's save, read-only, card list omitted. On a phone: `adb forward tcp:18384 tcp:18384` (SSGF: 18385), then open `http://127.0.0.1:18384/save` |
| `GET …/manifest/<version>.txt` | `games/<game>/manifest.json`, with `ServerUrl` = `--public-url` and `Cryption` = 0 (plain JSON responses) |
| `GET /cdn/<host>/<path>` | Files from `<data>/cdn/`, laid out like the original URLs. Encrypted master data tables are decrypted, their URLs rewritten to this server, and re-encrypted (cached in memory). Missing `_Zoom` card frames fall back to `_Large`; any other missing image is answered with a 1×1 transparent PNG, because the game's launch download refuses to finish if any file fails |
| `POST` (anything else) | RPC: `{"class", "method", "params"}`, plain or AES-CBC encrypted (`X-SuperStar-AES-IV`); answers `{"code": 1000, "result", "invoke": []}` |

**URLs.** The server rewrites every original CDN/S3 URL it hands out (manifest, asset context, table contents)
to `<public-url>/cdn/<host>/…`, so the game never contacts the internet for data.

**API.**
- Calls with a handler in `src/api.rs` get real answers: startup, account and profile, tutorial, cards, card book,
  card packs, songs and results, lobby wallpapers and artist packs.
- Every other known call gets its default from `<data>/api-defaults.json`.
  These defaults are generated from the game's model classes (empty lists, zeros, nested objects).
- Unknown calls get `null`.
- Wrong shapes found while testing are fixed in `games/<game>/api-overrides.json`,
  which `tools/build-server-data.py` merges over the generated defaults. No Rust change is needed.
- Each call is logged with `[handler]`, `[default]` or `[UNKNOWN]`,
  so testing with the real app shows what still needs a proper handler.

**Master data** can be changed in `games/<game>/table-overrides.json` (e.g. Jiho, a former member, is shown like a current
member; real-money store content is removed).
`tools/build-server-data.py` writes changed tables as new files into the server data (the mirror stays original).
Their version goes up whenever the content changes (`games/<game>/table-versions.json`),
so installed games download them again.

**Card progression** (`src/progression.rs`, numbers in `games/<game>/progression.json`,
design in `games/ssomg/re/notes/06-progression.md`):
- The tutorial gives a normal 30-pack, opened after the profile picker.
- Every song pays RP; the first clear of a song gives a card.
  Song-count milestones and a one-time record ladder add RP, diamonds, cards and packs.
- Packs are bought with RP or diamonds; power-up, grade-up, selling and the card book follow the game's tables.

**Player.**
- `--save` is a JSON file, created on first run.
- Nickname "Miracle", object ID 1. Starts with the balances from `progression.json`
  (0 RP, 0 diamonds, unlimited headphones) and owns every profile picture, lobby wallpaper and artist pack.
- Saves from before card progression (version 1) are moved aside and replaced.
  A save past the tutorial without cards gets the starter pack on load.
- Best score per song and difficulty.
- Login always succeeds, whatever credentials or nickname the client sends.

**Data.** `--data` is a game's `work/<game>/server-data/` folder or an uncompressed zip of it.
It can be given several times: the data split APKs (`asset-slices/gamedata*-master.apk` in the .apks) work too,
with the tree under `assets/gamedata/` (data packs built before the multi-game layout: `assets/ssomg/`).

## Source layout

| File | Purpose |
|---|---|
| `src/main.rs` | CLI |
| `src/lib.rs` | `Config`, `run()`, HTTP routing |
| `src/http.rs` | Minimal HTTP/1.1 server: a thread per connection, keep-alive |
| `src/api.rs` | RPC dispatch and handlers |
| `src/assets.rs` | Manifest, asset context, URL rewriting, local CDN |
| `src/progression.rs` | Card progression: packs, power-up, grade-up, selling, card book, song rewards |
| `src/state.rs` | Player save file |
| `src/crypto.rs` | AES-256 table (ECB) and API (CBC) formats |
| `src/data.rs` | Folder or zip data source (several zips: the data splits) |
| `src/android.rs` | Start inside the game on Android |

## Android build (server inside the game)

```bash
tools/build-android.sh ssomg                 # NDK cross-compile -> build/ssomg/libss.so
tools/build-apk.py --game ssomg --embedded   # adds lib/arm64-v8a/libss.so, points the manifest at 127.0.0.1:<port> (game.json "port")
tools/build-apks.py --game ssomg             # app bundle + bundletool -> build/ssomg/ssomg-offline.apks
```

**How the library gets loaded.**
The game loads `libssnative.so` only lazily, through a P/Invoke call, so that library can't host the server.
Instead, `tools/build-apk.py` renames the `libdl.so` dependency of Unity's `libmain.so`
(the first native library the app loads) to `libss.so`, a same-length rename.
Our library depends on libdl/libm itself, so nothing goes missing.

Two linker details:
- The library is linked with SONAME `libss.so`, because Android matches dependencies by SONAME.
- libmain's symbol-version requirement now names `libss.so`.
  Bionic accepts that because our library defines no versions.

**What runs on the phone.** An ELF constructor (`src/android.rs`) starts the server on `127.0.0.1:<port>` (each game its own: 18384 SSOMG, 18385 SSGF; from `server.json` in the data),
in the main process only:

| | Path |
|---|---|
| Data | `<app dir>/split_gamedata0.apk`, `split_gamedata1.apk`, ... (app dir from `/proc/self/maps`; or a `/sdcard/Android/data/<pkg>/files/server-data/` folder) |
| Save | `/data/user/0/<pkg>/files/offline.save.json` (an older `ssomg.save.json` is renamed to it) |
| Log | `/sdcard/Android/data/<pkg>/files/server.log` |

**Packaging** (`tools/build-apks.py`).
The game and its data ship as one `.apks` file, the split APK set format of Google's bundletool:
- An app bundle is assembled from the patched APKs (converted back to bundle layout with `aapt2 convert`)
  plus install-time asset packs `gamedata0`, `gamedata1`, ... holding the server data uncompressed under `assets/gamedata/`.
- The packs are at most 1.5 GB each:
  Android can't verify an APK over 2 GB ("APK Signature Scheme v3: integer overflow").
- `bundletool build-apks` makes the `.apks`
  (`toc.pb`, `splits/base-master.apk`, `splits/base-arm64_v8a.apk`, `asset-slices/gamedata<N>-master.apk`).
  Per-Android-version variants are switched off, so there's exactly one set of APKs, which keeps manual installs simple.
- bundletool comes from the Gradle cache (or `BUNDLETOOL=bundletool-all.jar`).

No OBB, no storage permission: the installed splits sit next to `base.apk`.

**Install:** with a split APK installer (open the `.apks`), or:

```bash
cd build/ssomg && mkdir -p apks-extract && unzip -o ssomg-offline.apks -d apks-extract
adb install-multiple apks-extract/splits/*.apk apks-extract/asset-slices/*.apk
```

When only the game changed, `tools/build-apks.py --game ssomg --skip-data` reuses the data packs.
`adb install-multiple -r -p com.dalcomsoft.ssom apks-extract/splits/*.apk` then updates only the base splits
and keeps the installed data splits (partial install).

## Not done yet

- **Response shapes:** many calls still get model-based defaults.
  The call log decides which ones need handlers or `games/<game>/api-overrides.json` entries.
