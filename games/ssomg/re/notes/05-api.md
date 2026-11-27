# Game server API and login flow

Generated references:

- `games/ssomg/re/api/catalog.md` / `catalog.json`: all 196 API wrappers (`tools/api-catalog.py`)
- `games/ssomg/re/api/models.json`: 295 request/response model classes, enums and constants, with JSON
  field names (`tools/models.py`)
- `games/ssomg/manifest.json`: the startup manifest's keys, with only the values the offline game uses

## 1. Finding the server: the manifest

`NetworkControl.Open` runs at startup and downloads a JSON manifest:

```
https://s3.ap-northeast-2.amazonaws.com/superstar-ohmygirl/version/real/manifest/<version>.txt?t=<unix ms>
```

- The URL prefix (`manifestHead`) is serialized in the `KeyVault` asset
  (`work/ssomg/unpacked/base/assets/bin/Data/28703887e748b0d46b12dd4e1ae83bbc`), not in code.
- The rest is built by `KeyVault.NetworkControlData.GetManifestURL`.
- The original manifest contained:

| Key | Value | Used for |
|---|---|---|
| `ServerUrl` | `https://ssom-api-https.dalcomsoft.net/` | All API calls (`Client.endpoint`) |
| `Cryption` | `"0"` | `Client.isDecrypted`: response bodies are encrypted only if 1 |
| `ActiveVersion_Android` | `3.22.4` | Forced-update check |
| `MaintenanceUrl` | CDN page `ssom_popup_end_of_service.html` | Maintenance/shutdown popup |
| `TitleBackgroundUrl` / `Count` | CDN, 20 images | Title screen |
| `TermsVersion`, `DateOfBirth`, `ProbabilityUrl`, `OfferwallSuffix`, `TesterIp`, `DalcomAccountDev` | | Minor |

`tools/build-apk.py` replaces `manifestHead` in the KeyVault asset with the local server's address,
and the server answers with a manifest whose `ServerUrl` is itself. No code patch is needed for the
API address.

## 2. Wire format

Code: `com.dalcomsoft.project.client.Client` and `Client.NetworkHandler`.

- One endpoint for everything: `POST <ServerUrl>`.
- Request headers:

| Header | Value |
|---|---|
| `X-SuperStar-AES-IV` | Random 16-character string, regenerated per request (`Client.randomString`); the IV is its UTF-8 bytes |
| `X-SuperStar-API-Version` | `Client.apiVersion` |
| `X-SuperStar-Class-Version` | Per-call class version (often empty) |
| `X-SuperStar-Asset-Version` | Client's master-data version (`a.json` `version`) |
| `X-SuperStar-Asset-Ignore` | `true` / `false` |

- **Request body:** JSON `{"class": "<clazz>", "method": "<method>", "params": [...]}`
  (model `protocol.Request`).
  - If `Client.isEncrypted` is set, the body is base64 of
    AES-256-**CBC**/PKCS7(JSON), with the same key as the master data (`tools/gamekey.py`) and the header IV.
  - The key comes from `libssnative.so` (`ssnative_copys(3549345)`) and is the same one
    the master data uses.
  - `libssnative.so` holds three more 32-character keys (IDs 3248567, 5908235, 7145493);
    their use is not yet known.
- **Response body:** JSON `{"code": 1000, "result": <any>, "invoke": [{"method": ..., "params": [...]}]}`
  (model `protocol.Response`).
  - Encrypted the same way only when the manifest says `Cryption: 1`.
  - `code` 1000 = OK. All codes are in `ResponseCode` in `models.json` (157 constants:
    -2002 asset update required, -3000 token expired, ...).
  - `invoke` lets the server push extra client-side calls (e.g. currency or flag updates);
    it can be empty.
- `params` is the wrapper's C# parameters in order. Many calls put the user's object ID
  first; `catalog.md` marks these with `uid`.
- Unknown: the order inside `params` isn't verified for every call yet, and the setter of
  `Client.isEncrypted` hasn't been found. A local server should accept both a plain and an
  encrypted body.

## 3. Startup and login sequence

`TitleScene` state machine (`TitleScene.STATE`): NetworkStatusCheck → (SelectLanguage, Intro,
TermsAndPolicy, Tutorial) → Loading → SDKInitialize → LoadingDone → Main → login → NextScene.

1. **Manifest** (`NetworkControl.Open`), then server status and time (`Platform.getTimestamp`
   via `TimeControl`), region (`Platform.getRegion`) and client IP (`Platform.getClientIP`).
2. **App hash check** (`AppHashChecker`, calls `Platform.getClientHash`). Skipped by our patch.
3. **Assets** (`AssetControl` / `AssetDownloader`): `Platform.checkAssetBundle` returns an
   `AssetContext` = `{"version": N, "context": {"MusicData": {"file": url, "version": n}, ...}}`.
   This is the same format as `games/ssomg/re/captures/asset-context-3.22.4.json`. The client then downloads
   any changed tables.
4. **Login**:
   - Guest:
     1. `Account.createAuthentication` returns `Authentication {userObjectID, provider, userID, localPassword}`.
     2. `Account.createAccount(provider=GUEST(0), userID, localPassword, nickname, language, channel)`.
     3. `Account.login(...)`.
   - Returning player: `LoginControl.LoginAuto` calls `Account.login` with stored credentials.
   - `Account.login` returns `LoginResponse`. Its `user` field is the `Player`; the full
     field list is in `models.json`.
   - Other paths: membership, Google, Apple, Dalcom account (the latter on
     `oauth.dalcomsoft.net`, a separate REST API in `DalcomAccount`). The offline game needs
     only guest or auto login.
5. **Post-login data load** (`LoginControl.LoginStep1..28`, run in order):

| Step | What | API call |
|---|---|---|
| 1 | Delete old news files | (local) |
| 2 | Card inventory | `CardInventory.getCardList` |
| 2_2 | Live themes | `LiveThemeBook.getList` |
| 3 | Stage status | `Game.getStageStatus` |
| 4 | Card book | `CardBook.getList` |
| 5 | Consumable items | `Store.getConsumableItemCount` |
| 6–7 | In-app purchase init, pending purchases | (store SDK, local) |
| 8 | Active effects/buffs | `UserControl.UpdateMyEffects` |
| 9 | Weekly league rank | `LeagueWeekly.getRank` |
| 10 | SuperStar league rank | `SuperstarLeagueControl.GetRank` |
| 11 | Purchase events | `Store.getPurchaseEvent` |
| 12 | Card book registration | `CardBook.regist` |
| 13 | Store buy counts | `Store.getBuyCount` |
| 14 | Bought profile images | `Account.getBuyProfileImage` |
| 15 | Collect event | `Promotion.getCollectEventInfo` |
| 16 | Event missions | `EventMissionControl.GetEventMissionList` |
| 17 | Missions | `MissionControl.GetMissionList` |
| 18 | Star pass | `StarPass.getPassStatus` |
| 19 | Arena rank | `ArenaControl.GetRank` |
| 20 | Advertising ID | `UserControl.ChangeADID` (calls `Account.changeADID`) |
| 21 | Card pack points | `Store.getStoreCardPackPoint` |
| 22 | My store | `Store.getMyStoreInfo` |
| 23 | Artist packs | `Store.getMyArtistPack` |
| 24 | Profile info | `Account.getMyInfo` |
| 25 | Music contents unlocks | `Account.getUserMusicContentsStatus` |
| 26 | Music content missions | `MusicContentsMissionControl.GetContentMissionList_AllTabCheck` |
| 27 | Buffs | `Account.getBuffStatus` |
| 28 | Weekly store | `Store.checkWeeklyBuyCount` |

Steps whose API call is a `*Control` name go through a delegate that the call-graph scan
can't follow; `catalog.md` lists the matching wrappers.

## 4. Playing a song

- `Game.stageStart(uid, musicCode, difficult, isADFree)` returns `Game.StageStartResponse
  {musicID, challenge, yearMonth, startedAt}`.
- `Game.stageComplete(uid, score, miss, good, perfect, sPerfect, combo, leaderCardObjectID,
  missionVerify, collectCount, missionGood, missionPerfect, missionSPerfect, cardScoreJson,
  longNoteCountJson, isActiveCollectEventBuff)` returns the result and rewards.
- Rehearsal, arena and SuperStar league have their own start/complete pairs.

## 5. First launch and login

There is one player, **"Miracle"**, who always auto-logs in. A fresh install goes:
logo → tutorial question → resource download → profile select → lobby. There are no
terms, consent, permission, login-method or nickname popups.

How the title screen decides (`TitleScene` / `LoginControl`):
- `LoginControl.IsAccountExists` = `PlayerPrefs.GetString("vjckkjd")` (account code) is
  not empty.
- `IsLoginAvailable` = `PlayerPrefs` `"gsdkln23r8yu9"` (auth object ID) is not 0.
- The provider is stored under `"adkjsad"`.
- If both are true, the title goes to `LoginAuto` and then `Account.login`. Otherwise it
  shows the login popup and then the guest nickname flow.

The patches that do it (all in `games/ssomg/patches.json`):

| Patch | Effect |
|---|---|
| `account-always-exists` | `get_AccountCode` passes the key `"vjckkjd"` as `GetString`'s default, so a fresh install has an account code |
| `login-always-available` | `get_IsLoginAvailable` returns true |
| `title-skip-login-select` | Login-method popup jumps to guest login (fallback; not normally reached) |
| `skip-terms` | Terms/privacy screen skipped |
| `no-ad-sdk` | ADX ad SDK and its GDPR consent form never initialized |
| `grant-all-permissions` | `NetworkControl.CheckPermission` calls its callback right away |
| `skip-attendance-popup` | Daily attendance popup skipped (its table ends 2025-04) |
| `settings-noop-*` | Account buttons in Settings do nothing |
| `skip-app-hash-check` | Anti-cheat code hash skipped |

`tools/build-apk.py` also renames the prompt and ad permissions in `AndroidManifest.xml`
(`DISABLED_PERMISSIONS`), so Android refuses them without showing a dialog.

The download confirmations are skipped too (`skip-download-confirm`, `skip-song-download-confirm`,
`skip-extra-download-confirm`). The tutorial question (`TitleScene.SetStateTutorial`) is still asked.

**Server side:**
- `createAuthentication`, `createAccount`, `login` and `loginSNS` always succeed and return
  the same player. The login on a fresh install arrives as `[0, "vjckkjd", "", 2, "3.22.4", false]`.
- Nickname changes are stored and free; profile picture, lobby background and tutorial step are saved.

### Approach: binary patches + custom server

The app can't be recompiled: IL2CPP ships native ARM64 code, not C# source. All app
changes are therefore patches to `libil2cpp.so` (plus asset edits where needed, such as
the manifest URL in the KeyVault asset). Runtime hooking (Frida etc.) is ruled out.

- Most patches swap one 4-byte ARM64 instruction: `ret` turns a handler into a no-op,
  `b` jumps to another function.
- Larger changes are written as short instruction sequences in the same JSON format.
- `tools/apply-patches.py` checks the original bytes first.

A server-only approach isn't possible: the manifest and API are HTTPS, and Android apps
ignore user-installed CAs, so the APK has to change anyway.

## 6. What the offline server answers

- Implemented: manifest, `getTimestamp`, `checkAssetBundle`, guest/auto login with a full `Player`, the
  post-login calls, cards, songs, store and progression (`06-progression.md`).
- Everything else (purchase, pass, league, arena, events, missions, post box, promotions, ads) gets a
  default reply built from the response model (`api-defaults.json`, `tools/build-server-data.py`),
  or a hand-written one from `games/<game>/api-overrides.json`.
