# Card progression for the offline game

Goal: cards are earned through gameplay only, in a way that feels rewarding but not trivial.
The game's own tables stay the basis; outside references (other games' drop rates) are only a
secondary anchor. Implemented in `server/src/progression.rs`; all numbers in
`games/<game>/progression.json` (SSGF and SSLO use the same system with their own tables).

## The original economy (from the master data)

Reward item codes (`extra_resource` icons): 201 RP, 202 diamonds, 203–207 a card of grade
C/B/A/S/R, 208–211 normal packs of 1/5/10/30, 212–216 premium packs. Newer tables use item
types 2 = diamonds, 4 = RP.

| Source | Per month (approx., active free player) | Offline |
|---|---|---|
| Daily missions (`mission`, play 1/4/7/20 songs …) | ~4,500 RP + 1 B + 1 A card per day; some S cards, 10,000 RP, 50 diamonds | **Keep, as song-count milestones** |
| Record ladder (`myrecord`, records on 1–25+ songs per month) | up to 420 diamonds, 54,000 RP, normal 10-pack, premium 5/10/30-packs | **Keep, runs once (no monthly reset)** |
| Attendance (`attendance`, `attendance_consecutive`, playtime, comeback) | ~110 diamonds, ~74,000 RP, ~11 A cards, ~18 five-packs | Drop |
| Weekly league (`league_weekly_reward`) | 11–32 diamonds + 3,500–16,500 RP per week | Drop (maybe "songs of the week" later) |
| Star pass free track | 30 diamonds + 16,000 RP per ~29-day season | Drop |
| Collection achievements (200, one-time) | 5,645 diamonds + 2.1 M RP in total | **Keep as is** |
| RP per song | server-side; config has RP_BONUS 165, RP_BONUS_REVISION 15000, easy/normal/hard 18/45/90. Formula lost. | **Keep**, formula to be fitted |
| Tutorial | normal 30-card pack (`tutorialNormalCardPack30`); new players start with 0 cards | **Keep** |

Total ≈ 300,000+ RP, ~700 diamonds, 40+ A/B cards and several packs per month
(~2,000–2,500 RP-equivalent per song at 4–5 songs a day).

Per-song RP data point (SuperStar JYP, different constants): maxed deck, hard,
score 6,373,000 → 1,541 RP. So per-song RP was modest; most income came from missions,
attendance and the record ladder.

Prices: normal packs 5,000/10,000/30,000 RP for 5/10/30 cards; premium packs 50/90/240
diamonds; RP for diamonds 5,000 RP per 10; inventory slot 5 diamonds (200 slots at start);
nickname change 50; special store: theme exchanger, 100% power-up card, R/S card selectors.

## Decisions

- **Start:** 0 cards, the tutorial gives a normal 30-card pack.
- **RP from every song**, from the original constants (the formula itself is lost; a placeholder is used).
- **Missions → song-count milestones** (e.g. every 5 songs: RP + B card; every 20 songs:
  A card + 50 diamonds), amounts from the `mission` table.
- **Record ladder runs once**, cumulative over records set (tier rewards from `myrecord`).
- **Collection achievements unchanged** (one-time diamonds and RP).
- **First clear of a song** gives a card from that song's album/era (the song result has a
  `firstClearReward` field for this).
- **Diamonds stay**, earned only from these one-time milestones. Premium packs can be
  earned directly (record ladder) and stay rare, but collecting R cards must not become
  prohibitively grindy.
- **Packs:** game's own prices; fair odds with guarantees per pack size; dupes are power-up
  and grade-up material. Power-up and grade-up use the game's tables (`card_intensify`,
  `card_balance`, grade-up = two same-member same-grade cards).
- **Pacing:** relaxed. Target roughly the original's play-based income without
  attendance/league/pass. All numbers go in one config file.
- **Drop:** attendance, events, real-money store, ads, star pass, headphones/stamina (hide,
  or keep the balance very high if they can't be hidden).

## Implementation

**Client contract** (from the IL2CPP code):
- Rewards are `RecentProvide {tag, item, quantity, context, providableItemCode}`, which the
  client applies itself.
  - `item` is a PaymentCategory: 10 = free RP, 9 = free diamonds, 0 / 1 = normal / premium
    pack with the cards in `context` (a `List<Card>`).
  - Costs come back as negative provides (power-up returns one `provide`; there is no
    RP-sync invoke).
- Song result (`Game.stageComplete`): the result scene applies `reward` provides by tag:
  `music_play` (RP), `clearReward`, `tutorialNormalCardPack30`, plus bonus tags.
  `firstClearReward {cardID, level, grade}` (card 0 hangs the game) switches the result screen to
  the first-clear reroll flow (`CardRefreshControl.ProcessCardRefresh`): a face-down card with a
  3-second countdown, then `Game.getGotchaCard(gotchaCardObjectID)` delivers a `gotchaCard`
  provide, and `Game.refreshFirstClearCard` rerolls (limit: GameConfig `CARD_REFRESH_MAX_COUNT`,
  missing from our data). Offline the face-down card never reacted to taps, so the server sends
  `firstClearReward: null`. Without it, `ResultScene.SetStateResultReward` opens the `clearReward`
  provides with the normal card popup (`ProvideControl.Open`), which carries the first-clear card.
- `stageComplete` args: playID, score, miss, good, perfect, sPerfect, combo,
  leaderCardObjectID, …
- Stars (song list): the client counts them itself (`IngameScene.WriteResultData`): miss rate =
  miss / (miss + good + perfect + sPerfect) * 100 against MusicData `three/two/oneStarMaxMiss`
  (0 / 5 / 100 for every song). After a restart they come from `getStageStatus` records:
  `easy/normal/hardScoreGrade` = difficulty * 3 + stars - 1, plus `…AllPerfect`. The server
  keeps the best star count per song and difficulty.

**Starter pack:** a new save has 0 cards. After the tutorial
question (played or skipped), the profile picker sends `Game.tutorialStepComplete(1)`; its
reward list is opened by the client (`TutorialAssist.<OpenChangeProfilePopup>b__8_1` →
`ProvideControl.Open`), so the server returns the normal 30-pack there (tag
`tutorialNormalCardPack30`, once per save, `claimed.starterPack`) and you get the pack
opening. The end-of-tutorial dialog "Do you wish to proceed? (Reset Nickname & Cards /
Proceed)" is patched out (`skip-tutorial-proceed-dialog`): its reroll path logs in again
and hangs with our server. A save past the tutorial without cards gets the pack on load.

**Server side:**

| Call | Behavior |
|---|---|
| `Store.buyCardPack(code)` | CardStoreData row (codes 0–2 premium for diamonds, 3–5 normal for RP); pack odds and guarantees from the config; returns [pack provide, negative cost provide] |
| `CardInventory.intensify` | Wire order (the wrapper's object[], not its signature): target, materials (a JSON string), probRate, priceRate. Returns one CardIntensifyResult per material (0 success, 1 failed, 2 abort; a wrong order gave an empty list and crashed the result popup). Per material: success chance and RP price from CardIntensifyData[level code of target][material grade+level]; +1 level per success (max 5, R max 50); materials consumed; stats re-rolled from CardBalanceData |
| `CardInventory.upgrade` | Wire order: request, target, priceRate. Grade-up: same member, grade (CardGradeBonusValue) and level, material neither equipped nor locked (client `InventoryControl.HasUpgradeCard`) → the request card is upgraded in place (same objectID: the client looks it up after the reply; replacing it hung the game), next grade, level 1; only the material is used up; costs the two cards' sell prices |
| `CardInventory.sell / sells` | CardSellData price per grade as RP; locked cards are skipped |
| `lock / multiLock / equip / unequip / multi…` | Card flags |
| `CardBook.getList / regist / getReward` | Best grade per card ID; collection achievements pay out once |
| `Game.stageComplete` | RP (placeholder formula), first-clear card per song and difficulty (from the song's group) plus `firstClear.rhythmPoint` RP, song-count milestones, record ladder. The RP matters: the result screen stayed blank when a song's rewards were only card packs |

**Data details:**
- Level codes: C1..S5 = 0..19, R1.. = 20.. (rows of CardBalanceData / CardIntensifyData;
  material columns "C1".."R100").
- Pack pool: all 1,940 non-material cards, equal weight. The grade is rolled separately.
- Save version 2 (version-1 saves, with everything unlocked, are moved to `*.v1.json` and replaced).

**Simulated pace** (100 songs on normal, ~500k score):
- ~206k RP (~2,050 per song; original ~2,000–2,500), i.e. ~41 normal 5-packs.
- 670 diamonds, i.e. 2 premium 30-packs.
- 100 first-clear cards, plus 20 B / 5 A / 2 S milestone cards.
- R cards: ~3–4 within the first 25 songs (ladder), then ~1.7 per premium 30-pack plus
  grade-ups.

`tools/save.py` (server `GET/PUT /save/full`) backs up, edits or restores the save on a phone.

**Open:**
- Fit the RP formula to real result screens.
- Card book theme points / buffs (`themePointList`, `totalBuff`) are empty.
- Card selectors and the special store are not wired up.

## Store without real money

`games/ssomg/table-overrides.json` empties the real-money and ad tables (`RecommendStoreData`,
`DiamondStoreData`, `WeeklyStoreData`, `PopupStoreData`, `StarPassStoreData`,
`AdStoreData`, `HeadphoneStoreData`) and the store home banners, and removes every store tab
of those kinds plus the store home (tab 0, whose banners were all real-money offers).

**Crash fix for missing tab types** (libil2cpp patches):
- `StoreMenuControl.Open` builds a static `Dictionary<short,short>` store type → code of the
  first unlocked tab (rows with `unLock` false are skipped entirely).
- `StoreAssist.Start(type, code = -1)` looked the type up with `get_Item`, which throws
  `KeyNotFoundException` when no tab of that type exists. The game opens types 0 (lobby),
  1, 2, 5 and 16 from elsewhere, e.g. "not enough diamonds". Patch
  `store-missing-tab-no-crash` calls `TryGetValue` instead (code 0 when missing).
- `StorePopup.SetState(short code)` and `SetState(TYPE)` already fall back to the first tab
  of type 0 when the code or type is missing. Patches `store-fallback-card-packs-*` change
  that fallback to type 3, so any missing tab opens the card packs.

**Play time event and buffs:**
- The daily play time rewards (5–60 minutes per day, reset at midnight KST) are hidden via
  `AttendanceManagementData` row 2 (`playtimeAttendanceCode` → null). The play time is
  counted on the phone, so a non-daily variant isn't possible without patching the timer.
- The lobby's buff button is hidden by the patch `hide-buff-button` (buff tickets and
  events don't exist offline).
- The left lobby menu (Mission and Event: missions, post box events and event banners are
  empty offline; song-count milestones on the server replace the missions) is hidden by
  `hide-left-menu` + `hide-left-menu-arrow`: LobbyView.InitMenus always starts it closed and
  deactivates its arrow button. The Message button in the top bar is hidden by
  `hide-message-button`.

**Other tabs and owned items:**
- Removed tabs: burning tickets (4004) and level transfer tickets (4005), which were event items
  that can't be bought; My Store (6000; it polled `getMyStoreInfo` forever); inventory
  expansion (9000; the inventory size is 9999 from `games/ssomg/progression.json`, applied to every
  save on load); the pop-up shop (10000; its fixed button is hidden by
  `hide-popup-shop-button`).
- Lobby wallpapers (330) and artist packs (13) are all owned: `getMyLobbyBgList` and
  `getMyArtistPack` list everything. The store's wallpaper and artist pack tabs double as the
  inventory: Apply/Remove there, stored in `equipedLobbyBgList` and `artistPack`.
- An equipped artist pack whose files aren't all cached made the startup hang (step 23 tries a
  download it can't confirm); `no-artist-pack-download-at-login` skips that.
- Table versions go up whenever the overridden content changes (`games/<game>/table-versions.json`), so
  installed games download the changed tables again.
- The lobby news banner shows our own banner (`Promotion.getNewsData`, drawn by `tools/news_banner.py`).

Kept: card packs (2000/2001), RP exchange (3001), lobby wallpapers, artist packs.
