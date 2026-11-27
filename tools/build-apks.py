#!/usr/bin/env python3
"""Package the patched game and its data as a split APK set (.apks, bundletool's format).

Usage: tools/build-apks.py --game <id> [--skip-data]

Input:  build/<game>/apk/base.apk + config.arm64_v8a.apk (tools/build-apk.py --embedded)
        work/<game>/server-data/ (tools/build-server-data.py)
Output: build/<game>/<game>-offline.apks

How:
  1. An app bundle (.aab) is assembled from these modules:
     - base: the patched APKs converted back to bundle layout (aapt2 convert to proto
       format; manifest/, dex/, res/, assets/, lib/, root/).
     - gamedata0, gamedata1, ...: install-time asset packs with server-data/ under
       assets/gamedata/, stored uncompressed, at most PACK_LIMIT each (Android can't verify
       the signature of an APK over 2 GB: "APK Signature Scheme v3: integer overflow").
       The package manager installs them next to base.apk as split_gamedata<N>.apk, where
       the embedded server reads them (server/src/android.rs).
  2. bundletool build-apks turns the bundle into the .apks (toc.pb + splits/*.apk), signed
     with keys/<game>.keystore.

--skip-data reuses the gamedata modules from the previous run (build/<game>/bundle/gamedata*.zip),
which saves rereading ~2.7 GB when only the game changed.

Install: bundletool install-apks --apks=build/<game>/<game>-offline.apks, or any split installer
(SAI etc.), or adb install-multiple with the APKs from the .apks.
"""

import json, os, subprocess, sys, zipfile
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from game import ROOT, select

GAME, _ = select()
SDK = Path(os.environ.get("ANDROID_HOME") or sys.exit("set ANDROID_HOME to the Android SDK"))
BUILD_TOOLS = sorted((SDK / "build-tools").iterdir())[-1]
AAPT2 = BUILD_TOOLS / "aapt2"
ANDROID_JAR = sorted((SDK / "platforms").glob("android-*/android.jar"))[-1]
GRADLE_CACHE = Path.home() / ".gradle/caches/modules-2/files-2.1"
# bundletool and its dependencies, from the Gradle cache (Android Studio/Gradle put them there).
# BUNDLETOOL=/path/to/bundletool-all.jar overrides this.
BUNDLETOOL_DEPS = [
    "com.android.tools.build/bundletool",
    "com.android.tools.build/aapt2-proto",
    "com.google.auto.value/auto-value-annotations",
    "com.google.errorprone/error_prone_annotations",
    "com.google.guava/guava",
    "com.google.guava/failureaccess",
    "com.google.protobuf/protobuf-java",
    "com.google.protobuf/protobuf-java-util",
    "com.google.dagger/dagger",
    "javax.inject/javax.inject",
    "org.bitbucket.b_c/jose4j",
    "org.slf4j/slf4j-api",
    "com.google.code.gson/gson",
]


# bundletool writes multi-GB temporary zips; keep them next to the build, not in /tmp.
JAVA_TMP = f"-Djava.io.tmpdir={GAME.build / 'bundle/tmp'}"


def version_key(p):
    return [int(x) if x.isdigit() else 0 for x in p.parent.parent.name.replace("-", ".").split(".")]


def bundletool_cmd():
    if os.environ.get("BUNDLETOOL"):
        return ["java", JAVA_TMP, "-jar", os.environ["BUNDLETOOL"]]
    jars = []
    for dep in BUNDLETOOL_DEPS:
        found = [
            p for p in (GRADLE_CACHE / dep).glob("*/*/*.jar") if not p.name.endswith(("-sources.jar", "-javadoc.jar"))
        ]
        if not found:
            sys.exit(f"{dep} not in the Gradle cache; set BUNDLETOOL to a bundletool-all jar")
        jars.append(str(max(found, key=version_key)))
    return ["java", JAVA_TMP, "-cp", ":".join(jars), "com.android.tools.build.bundletool.BundleToolMain"]


KEYSTORE = GAME.keystore
KEY_PASS = f"{GAME.id}-offline"  # local signing key, see tools/build-apk.py
PACKAGE = GAME["package"]
DATA_PACK = "gamedata"  # + 0, 1, ...; must match DATA_SPLIT in server/src/android.rs
PACK_LIMIT = 1_500_000_000  # bytes per asset pack (Play's limit too; Android fails at 2 GB)
DATA_PREFIX = "assets/gamedata/"  # must match APK_PREFIXES in server/src/data.rs

APK_DIR = GAME.build / "apk"
WORK = GAME.build / "bundle"
OUT = GAME.build / f"{GAME.id}-offline.apks"

SIGNATURE_FILES = ("META-INF/MANIFEST.MF",)
SIGNATURE_EXTS = (".SF", ".RSA", ".DSA", ".EC")

DATA_MANIFEST = f"""<?xml version="1.0" encoding="utf-8"?>
<manifest xmlns:android="http://schemas.android.com/apk/res/android"
    xmlns:dist="http://schemas.android.com/apk/distribution"
    package="{PACKAGE}" split="{{split}}">
  <dist:module dist:type="asset-pack">
    <dist:fusing dist:include="true" />
    <dist:delivery>
      <dist:install-time />
    </dist:delivery>
  </dist:module>
</manifest>
"""


def run(*cmd):
    print("+", " ".join(str(c) for c in cmd)[:200])
    subprocess.run([str(c) for c in cmd], check=True)


def is_signature(name):
    return name in SIGNATURE_FILES or (name.startswith("META-INF/") and name.endswith(SIGNATURE_EXTS))


def module_path(name):
    """Where an APK entry goes in a bundle module."""
    if name == "AndroidManifest.xml":
        return "manifest/AndroidManifest.xml"
    if "/" not in name and name.startswith("classes") and name.endswith(".dex"):
        return "dex/" + name
    if name == "resources.pb" or name.split("/")[0] in ("res", "assets", "lib"):
        return name
    return "root/" + name


def build_base():
    """base module from the patched APKs. Returns the globs of entries to keep uncompressed."""
    proto = WORK / "base-proto.apk"
    run(AAPT2, "convert", "--output-format", "proto", "-o", proto, APK_DIR / "base.apk")
    uncompressed = []
    out = WORK / "base.zip"
    with zipfile.ZipFile(out, "w", zipfile.ZIP_DEFLATED) as dst:
        for src_path in (proto, APK_DIR / "config.arm64_v8a.apk"):
            with zipfile.ZipFile(src_path) as src:
                for info in src.infolist():
                    name = info.filename
                    if name.endswith("/") or is_signature(name):
                        continue
                    if src_path != proto and not name.startswith("lib/"):
                        continue  # the config split only contributes its native libraries
                    # Unity reads some assets in place and stores them uncompressed (noCompress).
                    if name.startswith("assets/") and info.compress_type == zipfile.ZIP_STORED:
                        uncompressed.append(name)
                    dst.writestr(module_path(name), src.read(name))
    return uncompressed


def data_packs():
    packs = [p for p in WORK.glob(f"{DATA_PACK}*.zip") if p.stem[len(DATA_PACK) :].isdigit()]
    return sorted(packs, key=lambda p: int(p.stem[len(DATA_PACK) :]))


def build_data():
    """gamedata<N> asset packs: server-data/ under assets/gamedata/, stored, PACK_LIMIT each."""
    for old in data_packs():
        old.unlink()
    data = GAME.server_data
    files = sorted(p for p in data.rglob("*") if p.is_file() and p.name != "CACHEDIR.TAG")
    packs, size = [[]], 0
    for p in files:
        n = p.stat().st_size
        if size + n > PACK_LIMIT and packs[-1]:
            packs.append([])
            size = 0
        packs[-1].append(p)
        size += n
    for i, pack in enumerate(packs):
        name = f"{DATA_PACK}{i}"
        src = WORK / f"{name}-manifest.xml"
        src.write_text(DATA_MANIFEST.replace("{split}", name))
        linked = WORK / f"{name}-manifest.zip"
        run(AAPT2, "link", "--proto-format", "--manifest", src, "-I", ANDROID_JAR, "-o", linked)
        with zipfile.ZipFile(WORK / f"{name}.zip", "w", zipfile.ZIP_STORED, allowZip64=True) as dst:
            with zipfile.ZipFile(linked) as z:
                dst.writestr("manifest/AndroidManifest.xml", z.read("AndroidManifest.xml"))
            for p in pack:
                dst.write(p, DATA_PREFIX + p.relative_to(data).as_posix())
        print(f"  {name}: {len(pack)} files, {sum(p.stat().st_size for p in pack) / 1e9:.2f} GB")


def main():
    (WORK / "tmp").mkdir(parents=True, exist_ok=True)
    uncompressed = build_base()
    if "--skip-data" not in sys.argv or not data_packs():
        build_data()
    (WORK / f"{DATA_PACK}.zip").unlink(missing_ok=True)  # single pack from earlier versions

    config = {
        "compression": {"uncompressedGlob": [DATA_PREFIX + "**"] + uncompressed},
        "optimizations": {
            # One split per ABI (only arm64 exists); densities and languages stay in base.
            "splitsConfig": {
                "splitDimension": [
                    {"value": "ABI"},
                    {"value": "SCREEN_DENSITY", "negate": True},
                    {"value": "LANGUAGE", "negate": True},
                    {"value": "TEXTURE_COMPRESSION_FORMAT", "negate": True},
                ]
            },
            # No per-Android-version variants (uncompressed libraries and dex for 9+, sparse
            # resource tables for 12+): one set of APKs for every device, so a manual install
            # only sees base, arm64 and the gamedata packs. Libraries get extracted at install, like the
            # APKs from tools/build-apk.py.
            "uncompressNativeLibraries": {"enabled": False},
            "uncompressDexFiles": {"enabled": False},
            "resourceOptimizations": {"sparseEncoding": "DISABLED"},
        },
    }
    (WORK / "BundleConfig.json").write_text(json.dumps(config, indent=1))

    aab = WORK / f"{GAME.id}-offline.aab"
    aab.unlink(missing_ok=True)
    run(
        *bundletool_cmd(),
        "build-bundle",
        "--modules=" + ",".join(str(p) for p in [WORK / "base.zip", *data_packs()]),
        f"--config={WORK / 'BundleConfig.json'}",
        f"--output={aab}",
    )
    OUT.unlink(missing_ok=True)
    run(
        *bundletool_cmd(),
        "build-apks",
        f"--bundle={aab}",
        f"--output={OUT}",
        f"--ks={KEYSTORE}",
        f"--ks-key-alias={GAME.id}",
        f"--ks-pass=pass:{KEY_PASS}",
        f"--key-pass=pass:{KEY_PASS}",
        f"--aapt2={AAPT2}",
    )
    aab.unlink()  # same size as the .apks; not needed afterwards
    with zipfile.ZipFile(OUT) as z:
        for i in z.infolist():
            print(f"  {i.filename:40} {i.file_size / 1e6:10.1f} MB")
    print(f"Wrote {OUT} ({OUT.stat().st_size / 1e9:.2f} GB)")


if __name__ == "__main__":
    main()
