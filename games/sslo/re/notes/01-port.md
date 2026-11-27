# SUPERSTAR LOONA 3.12.4: differences from SSOMG/SSGF

Same engine family (IL2CPP, same AES key, same table format and API shape), but an older Unity (2020.3).

## Game data: from the device cache, not the CDN

The CDN (`d3exy8s96shf22.cloudfront.net`) is gone; the S3 bucket still serves the manifest and all tables
(`games/sslo/s3-tables.aria2`). Everything else comes from a copy of the game's on-device cache
(`tools/import-device-cache.py`, game-specific paths in `games/sslo/device-cache.json`). There is no
`cdn-files.aria2`, so `build-server-data.py` places every file of `cdn-mirror/sslo/`.

**How the cache maps to URLs** (names from `AssetPathControl..cctor`): the download tree `asiudgasg/`
(found by `sdisgkdc/a.json`) holds
- `sdisgkdc/*.bin`: the tables (byte-identical to S3) and the cache indexes `url_shared_map.bin`
  (URL?v= → cache path), `url_version.bin` (cache path.ver → v=) and `url2.bin` (AssetURLData: URL?v= → size);
- `asodhsg/<folder>/<code>[_suffix][_lang].i.bytes`: images, named by the table row's code;
- `dslkjfsdalhh/<code>.a.bytes` (song), `aweid/<code>_4/_7/_13.seq` (charts),
  `dslkjfsdalhh/asdlkasdqoweirypqwier/<code>.a.bytes` (previews): by MusicData code;
- `asodhsg/kswoks/bg0..5.i.bytes`: the 6 title backgrounds (manifest `TitleBackgroundUrl`,
  `TitleBackgroundCount` 6; `EntryScene` needs real images, see SSGF's notes).

The URL map lists only part of the cache. The tool also learns the path pattern per table field from the map
(e.g. `CardData.cardImageLarge` → `asodhsg/wadceg/{code}_l.i.bytes`) and applies it to the other rows, and
places what's left by version + size when exactly one URL fits. Files on hosts the games share are taken from
the other games' mirrors (same URL, same file). Files only hidden or emptied tables use (`hiddenTables`,
`table-overrides.json`) are left out, language variants included. The server answers missing images with a
placeholder. Card frames have no `_Zoom` size; the server falls back to `_Large`.

`AssetPackData` points to the dead CDN and LOONA downloads it at the title (a 404 page there made the title fail
with JsonParseError); `build-server-data.py` serves such tables as an empty list with version + 1000.

## Server data

- `progression.json`: player "Orbit". Same structure and pack odds as SSOMG (LOONA's tables have no odds; same
  CardStoreData prices; no prism cards). Song milestones from LOONA's MissionData, record ladder from MyRecordData,
  RP constants and max levels from GameConfigData. Each `_why` says the source.
- `table-overrides.json`: store reduced to card packs (2000/2001), RP exchange (3001), wallpapers (7000/7001) and
  artist packs (8000); real-money tables emptied; the daily play time event hidden (AttendanceManagementData
  row 2); the live theme tables emptied (LOONA's only live theme is a leftover copy of SSOMG's "Fresher
  Edition", with OH MY GIRL's members). Relies on the store crash guards (`store-missing-tab-no-crash`, `store-fallback-card-packs-*`).
- `api-overrides.json`: `LeagueWeekly.getRank` as in SSGF; the news banner with LOONA's field names (`index`,
  `external`, `timeStartAt`, `timeEndAt`: LOONA's `News` has no `[JsonProperty]` names);
  `Store.getLimitedBuyCount` = [].

## Patches

`patches.json`: startup (auto-login, no terms, no login queue, no download prompts, device language, no ads,
Tapjoy or Facebook), then the UI cleanup like SSOMG's. Notes:
- File offset = VA in this binary (first LOAD segment at offset 0), unlike SSOMG/SSGF (VA - 0x1000).
- Unity 2020's delegate layout differs; callbacks are invoked through `System.Action$$Invoke` /
  `Action<bool>$$Invoke` instead of reading the delegate's fields.
- Message button: `TopViewControl.UpdateScene` has `RefreshMessageButton` inlined once per scene type, each
  patched to hide it (`hide-message-button-1..3`), plus the method itself.
- Store: `StoreAssist.RefreshAdButton` throws without ad data (ad SDK off) in the store popup's loaded
  callback, which left the store locked to input; it returns at once.
- Tapjoy connects from `TrackingControl.Open` and from its component's own Start; Facebook from
  `FacebookControl.Init` (`FB.Init`/`ActivateApp`; the SDK's own auto-init fails without a client token).
- Videos: AVPro Video 1.10.1's bundled ExoPlayer 2.8.4 crashed on an Android 17 phone (Adreno 750) during the
  first download (GPU driver crash, heap corruption on binder threads); the emulator never crashed.
  `avpro-use-mediaplayer` makes every AVPro player use Android's own MediaPlayer.
- Helpers in dead bodies: `TitleScene.SetStateTermsAndPolicy` (language helper),
  `InstallMusicAssist.OpenDownloadConfirmPopup` (hide-parent helpers, 0xF1908C-0xF190EF).

## Testing a full song

A temporary autoplay patch (not in `patches.json`): `nop` at 0x149C084 in `IngameView.Load` makes it always set
`IngameView.IsAutoPlay` (0x2C1), the game's own auto mode. `Game.stageComplete` carries no auto flag, so the
server treats it like a normal play.

## Open

- Grade-up and selling untested.
- `PlaytimeAttendanceControl.Open` throws a NullReferenceException at the end of loading (likely the hidden
  play-time event row); loading continues.
- The store's "Drop Rate" button does nothing (its web portal is gone).
- The title asks whether to play the tutorial song (skip-terms lands there); "Skip" works.
