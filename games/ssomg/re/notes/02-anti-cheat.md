# Anti-cheat (CodeStage Anti-Cheat Toolkit)

## What runs

**`AppHashChecker` (global namespace, obfuscated method names): the only active check.**

- Entry: `AppHashChecker.36a2c323(Action callback)` @ VA `0x1CC6DD8`, called from
  `TitleScene.f48d9d21` at startup.
- It checks the install source (`AppInstallationSourceValidator.GetAppInstallationSource`).
- It then builds a chain of steps. One step (`49b35c7b`) runs `CheckHash` async, which
  calls `CodeHashGenerator.GenerateAsync`. That hashes the APK's code on the device and
  is slow, which is likely part of the long startup.
- Another step (`06bd5695`) sends the hash with the version string and code to the server via
  `Platform.GetClientHash`. The server decides whether the hash is valid.
- On failure, `1648a0d5(string msg)` opens a popup via `ViewControl.OpenPopup`.
- `2e1a74c9(Action cb)` @ `0x1CC712C` simply invokes `cb` if it isn't null.

**Detectors (speed hack, time cheat, wall hack, injection, obscured cheating): not active.**

- No game code calls `StartDetection` or subscribes to their events.
- Their `RuntimeInitializeOnLoad` entries (`SubsystemRegistration`) only reset static
  state.
- No scene component was found that auto-starts them.

**Obscured types and `ObscuredFilePrefs`: harmless.** In-memory value obfuscation and
encrypted local prefs. Leave as is.

## Patch

`games/ssomg/patches.json` → `skip-app-hash-check`: the first instruction of `36a2c323`
becomes `b 0x1CC712C`, so the entry jumps straight to the callback.

## Re-checking

```bash
tools/xref.py --game ssomg 'CodeStage\.AntiCheat\.(Detectors|Genuine|Time)\.'   # game -> ACTk calls
tools/disasm.sh ssomg 0x1CC6DD8 0x1CC75A0                                 # AppHashChecker
```
