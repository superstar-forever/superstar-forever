# SuperStar Forever

Play Dalcomsoft's SuperStar rhythm games fully offline, without the official servers, which have been shut down.
Supported so far:

| Game | Id | Version | Package | Songs |
|---|---|---|---|---|
| SUPERSTAR OH MY GIRL | `ssomg` | 3.22.4 | `com.dalcomsoft.ssom` | 173 |
| SUPERSTAR GFRIEND | `ssgf` | 2.12.4 | `com.dalcomsoft.ssg` | 116 |
| SUPERSTAR LOONA | `sslo` | 3.12.4 | `com.dalcomsoft.sslo` | 91 |

This means all songs with an offline progression system, no in-app purchases, no ads,
and all cosmetics unlocked right from the start.
As a side-effect, this also removes the leaderboard, in-game events,
and annoying elements like attendance or play limits (headphones).
All game data is bundled with the app and served by a small Rust server, so no internet is needed to play.
This repository only contains the tools, patches and server code.
The games themselves and any assets aren't included and need to be provided by the user.

> [!IMPORTANT]
> Only Android is supported (64-bit ARM phones). There is no iOS version.
> This project doesn't publish any APKs.
> If you come across patched versions of these games elsewhere, don't install them.
> They aren't from here and may contain anything, so build them yourself as described below.

> [!NOTE]
> Unsurprisingly, nearly all of this was done with the help of AI (using Claude Opus 5.5),
> since implementing this would otherwise have been extremely time-consuming and thus infeasible.
> Only the behavior was specified and tested by hand, while the code itself wasn't reviewed.
> There are no guarantees that it works for you, so please take it for what it is.

## Build it yourself

You need the original game (the version in the table above) and the files from its CDN,
which are still online at the time of writing for SSOMG and SSGF.
LOONA's CDN is gone: instead, you need a copy of the game's download cache from a phone that had
downloaded everything (`Android/data/com.dalcomsoft.sslo/files/`).

Requirements:
- Linux, [uv](https://docs.astral.sh/uv/) (Python 3.11+; `uv sync` installs `pycryptodome`, `UnityPy` and `Pillow`), `aria2c`, Java
- Rust (`cargo`)
- The Android SDK in `$ANDROID_HOME`, with NDK, build-tools (aapt2, zipalign, apksigner) and a platform
- bundletool: in the Gradle cache (any Android Studio/Gradle install has it),
  or set `BUNDLETOOL=/path/to/bundletool-all.jar`

Steps, from the repository root. Every tool takes the game's id; the examples use `ssomg`,
replace it with `ssgf` for SUPERSTAR GFRIEND or `sslo` for SUPERSTAR LOONA.

0. Install the Python packages into `.venv/` (once):
   ```bash
   uv sync
   ```
1. Put the original `.xapk` (e.g. `SUPERSTAR OH MY GIRL 3.22.4.xapk` or `SUPERSTAR GFRIEND 2.12.4.xapk`)
   into `original/` and unpack it:
   ```bash
   tools/unpack.sh ssomg "original/SUPERSTAR OH MY GIRL 3.22.4.xapk"
   ```
2. Download the game data into `cdn-mirror/<game>/`
   (SSOMG: about 1.9 GB, 12,866 files; SSGF: about 0.7 GB, 5,340 files).
   The list leaves out what only removed shop pages and hidden menus (events, missions, passes) use.
   Re-run the same command to resume or retry:
   ```bash
   aria2c -i games/ssomg/cdn-files.aria2 -c -j 8 --auto-file-renaming=false
   ```
   For LOONA, download the tables (still online) and rebuild the rest of `cdn-mirror/sslo/` from the
   device cache (about 0.7 GB; the tool reports what it couldn't place):
   ```bash
   aria2c -i games/sslo/s3-tables.aria2 -c -j 8 --auto-file-renaming=false
   uv run tools/import-device-cache.py --game sslo path/to/files
   ```
3. Build the server data, the server for Android, the patched game and the `.apks`:
   ```bash
   uv run tools/build-server-data.py --game ssomg
   uv run tools/build-android.sh ssomg
   uv run tools/build-apk.py --game ssomg --embedded
   uv run tools/build-apks.py --game ssomg
   ```
   The first `build-apk.py` creates a signing key in `keys/<game>.keystore`.
   Keep it: updates must be signed with the same key.
4. Install `build/<game>/<game>-offline.apks` with a split APK installer on the phone (SAI, MiXplorer, ...),
   or with adb:
   ```bash
   cd build/ssomg && mkdir -p apks-extract && unzip -o ssomg-offline.apks -d apks-extract
   adb install-multiple apks-extract/splits/*.apk apks-extract/asset-slices/*.apk
   ```

Only 64-bit ARM phones (arm64-v8a) are supported.
Several games can be installed side by side: each one runs its server on its own local port
(`port` in `games/<game>/game.json`).

## Folders

In the repository:
- `tools/`: build and reverse-engineering scripts, shared by all games;
  `tools/fonts/` has Montserrat (SIL Open Font License, `tools/fonts/OFL.txt`) for the news banner
- `server/`: the replacement server, shared by all games (`server/README.md`)
- `games/<game>/`: everything specific to one game:
  `game.json` (package, version, key hash, manifest URL, server port), `patches.json` (byte patches for
  `libil2cpp.so`), table and API overrides, progression settings, the manifest template, the download list,
  and `re/` with reverse-engineering notes, the generated API catalog and the game's table index

Not in git:
- `original/`: the original `.xapk`/`.apk` files you provide (any name)
- `cdn-mirror/<game>/`: the downloaded game files, laid out like the original URLs (`<host>/<path>`)
- `keys/<game>.keystore`: the signing key; back it up together with `original/`
- `build/<game>/`: patched files, APKs and the final `.apks`
- `work/<game>/`: what the tools recreate: `unpacked/` (the game's contents), `server-data/` (what the server
  serves, assembled from the mirror with the overrides applied), `extracted/` (decrypted tables) and `re/`
  (IL2CPP dump and other reverse-engineering output)

## Tools

Python tools take `--game <id>`, shell tools take the game as their first argument (or `GAME=<id>`).
Run them with `uv run` (as in the build steps) or with `.venv/` activated.

```bash
tools/unpack.sh ssomg original/FILE.xapk             # .xapk/.apk -> work/ssomg/unpacked/
tools/dump-il2cpp.sh ssomg                           # Il2CppDumper -> work/ssomg/re/il2cpp-dump/, then index-calls.sh
tools/index-calls.sh ssomg                           # objdump call index -> work/ssomg/re/calls.txt
tools/xref.py --game ssomg 'AppHashChecker\$\$'      # who calls methods matching a regex
tools/disasm.sh ssomg 0x1CC6DD8 0x1CC75A0            # disassemble a range with method names
tools/literals.py --game ssomg 0x1EB2408 0x1EB26A0   # same, with string literals and types resolved
tools/apply-patches.py --game ssomg                  # games/ssomg/patches.json -> build/ssomg/libil2cpp.so
tools/decrypt-masterdata.py --game ssomg             # tables -> work/ssomg/extracted/masterdata/<Table>.json
tools/make-aria2.py --game ssomg                     # maintainer: rewrite games/ssomg/cdn-files.aria2
tools/import-device-cache.py --game sslo DIR          # CDN gone: rebuild cdn-mirror/sslo/ from the game's on-device cache
tools/api-catalog.py --game ssomg [--rescan]         # all API calls -> games/ssomg/re/api/catalog.{md,json}
tools/models.py --game ssomg                         # API model classes, enums, JSON names -> games/ssomg/re/api/
tools/build-server-data.py --game ssomg              # assemble work/ssomg/server-data/ for the server
tools/news_banner.py --game ssomg                    # preview the lobby news banner -> work/ssomg/news-banner.png
tools/test-client.py --game ssomg [URL]              # smoke-test a running server
tools/save.py --game ssomg pull|push|boost           # back up, edit or restore the save on a phone
tools/build-android.sh ssomg                         # server for Android arm64 -> build/ssomg/libss.so
tools/build-apk.py --game ssomg --embedded           # APKs with the server inside -> build/ssomg/apk/
tools/build-apk.py --game ssomg --server URL         # APKs for a server on the PC: http://127.0.0.1:18384 + adb reverse
tools/build-apks.py --game ssomg                     # game + data splits -> build/ssomg/ssomg-offline.apks
```

Building needs only what "Build it yourself" lists.
The reverse-engineering tools also need `il2cppdumper` and `aarch64-linux-gnu-objdump`.

## Status

All three games run fully offline from one `.apks` (game, server and data): auto-login, tutorial, lobby,
all songs and scores, card progression with packs, and a shop reduced to card packs and the RP exchange.
Tested on a phone and an Android 17 emulator.
How the progression works: `games/ssomg/re/notes/06-progression.md`; per-game details in `games/<game>/re/notes/`.

## License

The tools, patches and server code are under the MIT license (`LICENSE`).
The games, their names, data and assets belong to Dalcomsoft and the respective rights holders and are not covered by it.
