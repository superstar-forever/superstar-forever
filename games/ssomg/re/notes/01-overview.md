# Overview

## App

- `com.dalcomsoft.ssom` 3.22.4 (version code 118), split APK: `base.apk` + `config.arm64_v8a.apk`.
- Unity 2022.3, IL2CPP, metadata version 31. Il2CppDumper works without extra steps;
  its "This file may be protected" warning is a false alarm (the dump is complete).
- Most names are readable. A name obfuscator was applied to a few sensitive classes
  (e.g. `AppHashChecker` methods are named like `36a2c323`).
- HTTP library: BestHTTP. Network command classes live in `com.dalcomsoft.project.client.command`.
- No custom Java-side signature check (only Firebase/FCM and SDK code under `com.dalcomsoft`).

## Servers

- Game API: `ServerUrl` from the startup manifest (`05-api.md`).
- Accounts: `https://oauth.dalcomsoft.net/v1/...` (`token`, `account`, `account/info`,
  `account/verify`, `user`, `page/games`, ...). `oauth-dev.dalcomsoft.net` also present.
- Master data: `https://s3-ap-northeast-2.amazonaws.com/superstar-ohmygirl/version/real/asset/...`
  (index in `games/ssomg/re/captures/asset-context-3.22.4.json`).
- Game files: `ddlptvlpr3a8n.cloudfront.net` (`04-cdn.md`).
- Other: `failedsong.dalcomsoft.net/add`, `fb.dalcomsoft.net/cloud-messaging/...`.
- Login is a multi-step sequence (`LoginStep1` .. `LoginStep28` in log strings).

## Device cache

The game's download cache on the phone (`Android/data/<package>/files/`) uses obfuscated folder names
(the same in all three games, from `AssetPathControl`):

| Folder | Contents | Format |
|---|---|---|
| `asiudgasg/dslkjfsdalhh/` | Song audio (`<code>.a.bytes`), previews in `asdlkasdqoweirypqwier/` | Plain OGG Vorbis |
| `asiudgasg/asodhsg/` | Cards, jackets, UI images | Plain PNG |
| `asiudgasg/aweid/` | Note charts (`<code>_<4\|7\|13>.seq`) | Binary |
| `asiudgasg/soidhskd/` | Videos | WebM |
| `asiudgasg/sdisgkdc/` | Master data tables (`*.bin`) + `a.json` index | Like the CDN tables (`03-master-data.md`) |
| `il2cpp/` | Copy of `global-metadata.dat` + resources | Identical to the APK's |
| `Unity/` | Unity analytics cache | Not needed |

`tools/import-device-cache.py` rebuilds a CDN mirror from such a cache (used for SSLO, whose CDN is gone).
