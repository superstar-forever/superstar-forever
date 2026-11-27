# SUPERSTAR GFRIEND 2.12.4: differences from SSOMG

GFriend is the same Dalcomsoft engine as SSOMG (Unity 2022.3, IL2CPP, same AES key, same API shape),
so the SSOMG server and tools work with a few per-game settings. This lists what is different.

## Startup

- **Manifest URL**: a string literal in `global-metadata.dat`, not in a KeyVault asset
  (`manifestHeadFile` in `game.json`). It appears twice; `build-apk.py` replaces every copy.
- **Original manifest** (`.../superstar-gfriend/version/real/manifest/2.12.4.txt`) has an extra
  `"Liapp": "1"` (LIAPP anti-tamper), which the 2.12.4 client doesn't read; our `manifest.json` leaves it out.
- **No app hash check**: there is no `AppHashChecker`, so no patch for it.
- **Title backgrounds**: `EntryScene` waits until `BackgroundImage` has loaded a random title
  background (`TitleBackgroundUrl`, 9 images). The 1×1 placeholder the server hands out for
  missing images isn't accepted, so the images have to be in the mirror
  (`d194rwsjdpmwvw.cloudfront.net/resources/images/title_background/250107/`).
- **Tables**: the game wants the v204 index (`re/captures/asset-context-2.12.4.json`); one entry,
  `CdnAddress`, points to another publisher's CDN with no file name and is never downloaded.
  Table URLs contain `asset//` (double slash).
- **File names with non-ASCII characters** (e.g. `回LABYRINTH.png`) arrive percent-encoded; the
  server decodes them.
- **Login queue**: `TitleScene.RequestSequentialLogin` asks `ssg-login.dalcomsoft.net/enter/...`
  (gone) before logging in; patched to continue right away (`no-login-queue`).

## API

- Many response handlers also read `ArenaBasicInfo`/`ArenaLeagueReward`, so the API catalog lists
  them first; `build-server-data.py` skips them when picking the default result type.
- `LeagueWeekly.getRank` (`api-overrides.json`): needs `lastWeekAverage` for every league class
  and the player's own entry in `ranking`. The client finds "my" entry by object ID *and*
  nickname (`Ranking.IsMe`); without it `LeagueControl.MyRanking` is null and the lobby stays
  empty. The server fills in the player's name and leader card.
- `Promotion.getAdminInfo` (GFriend only): the lobby's reply handler sets up the ad timer and
  button and throws offline; patched to skip to the next lobby step (`lobby-skip-ad-info`).
- The player name is "Buddy" (`progression.json` → `start.nickname`).

## Patches

`patches.json`: the SSOMG patches that map 1:1, the startup fixes above, and the UI cleanup:

- **Shop**: the tabs are fixed in `StorePopup` (no StoreMenuData table). Recommended, Diamond,
  Headphone and My Card (inventory slots) are hidden, the pop-up shop button too; every other
  store type opens the card packs. Left: Rhythm Point (RP for diamonds, `Store.buyRhythmPoint`)
  and Card Pack. The tabs keep their original places, so there are gaps.
- **Lobby**: left menu (missions only) and the Message button (empty offline) hidden; in the ☰
  menu, Attendance and Notice are hidden (both did nothing offline), Help and Settings are left.
- Helper code for these lives in the dead bodies of `TitleScene.SetStateSelectLogin` and
  `InstallMusicAssist.OpenDownloadConfirmPopup` (both skipped by other patches).
- **Settings / My Info**: GFriend's settings have no account section; the account box in My Info
  (Dalcom ID banner, guest notice, delete account, sign in, clear data) is hidden.

- **Language**: the game keeps it in PlayerPrefs "language", defaulting to the build language
  (Korean); only the skipped language step set it from the device. Now the default is the device
  language (English for languages the game doesn't have); a choice in the settings is kept.

- **Prism cards**: half of the cards are prism versions (`CardData.prism`); the client shows them
  as prism from the table. They drop from all packs, and the record ladder's prism packs are real
  prism packs (server pack kind "prism": prism cards only, PaymentCategory CARD_PACK_PRISM 64-67).

## Live themes

GFriend's four live themes were rewards of an event (EventMissionData event 77: songs from 1/4/8/12 albums with
4,500,000+ points); none were sold. Offline (`progression.json` → `liveThemes`): an album counts once every song on
it has three stars (no miss) on Normal or Hard (`minScore` can add a score requirement),
and 1/4/8/12 complete albums unlock NEW LABYRINTH, SUMMER, 6TH ANNIVERSARY and ANIMAL. Unlocked themes stay. The server sends the unlock as provides 57/58/59 (cards, moving photos, voices), but the result screen
ignores them; the client reads `LiveThemeBook.getList` at login, so a new theme shows up after the next start.
`LiveThemeBook.setLeader` (main theme), `equipCard` and `equipBackground` are stored.

## Debugging without logs

GFriend's Unity log is silent (no exceptions in logcat). What worked: a temporary `brk #0` at an
address shows up as a native crash in logcat if the code is reached; bisecting a method with a few
of those finds the call that throws. The server option `SUPERSTAR_LOG_CDN=1` logs every CDN file
it serves (normally only misses are logged).
