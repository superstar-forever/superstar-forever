# CDN

The game files are on `ddlptvlpr3a8n.cloudfront.net`, the tables on S3 (`03-master-data.md`).

## Folder structure on the CDN

| Path | Content |
|---|---|
| `resources/sounds/<artist>/<song>/` | Songs from launch: `.ogg` audio, `_4/_7/_13.seq` charts |
| `resources/images/<kind>/` | Launch-era images: `card`, `icon`, `theme`, `card_edge`, `store`, `music_background_image`, ... |
| `resources/live/<year>/<release date>/` and `resources/images/live/2022/<date>/` | Everything added later through live updates, grouped by release date: new songs and charts, cards, profile images, lobby backgrounds, theme name images, store products/banners, event/mission banners |

Tables name the base file (e.g. `shop_bg_x.png`). The game adds a language suffix at runtime
(`shop_bg_x_koKR.png`, `_enUS`, ...). These variants are in `url_v2.json` (the size index),
but no table names them directly.

## Download list

`tools/make-aria2.py` writes `games/ssomg/cdn-files.aria2`. Each entry saves to
`cdn-mirror/ssomg/<host>/<URL path>`, so the folder mirrors the game's URLs, with two exceptions:
- The `?v=` query is dropped. Where several versions of one path exist, the version a table references
  wins, otherwise the newest.
- Double slashes collapse to one (the S3 master data URLs contain `asset//`).

The list leaves out what only hidden menus and removed shop pages use (`hiddenTables` in `game.json`).

## Files that don't exist

Many URLs the tables name answer 403 (the file doesn't exist on S3); none of them have a size in `url_v2`,
so the game never had them either:
- Store and event graphics (`recommend_store`, `event_management`, `popup_store`, passes, other stores).
- Some language variants and locale-text images.
- `theme_type.grade*` card-frame base names: the game adds a size suffix at runtime (`_Large`, `_Small`,
  `_Collection`, `_Zoom`). `_Large`, `_Small` and `_Collection` exist for every frame, `_Zoom` only for a few;
  the server serves `_Large` for a missing `_Zoom`.

## Table index

The game uses table index version 536 (`games/ssomg/re/captures/asset-context-3.22.4.json`), from a device
dump; the server handed out 522 at shutdown (`asset-context-3.22.4-v522.json`). 536 adds song 19001 "Oh My",
24 cards and newer locale, profile, wallpaper and theme data. Table file names aren't content hashes, so a
newer index can't be reconstructed without such an `a.json`.
