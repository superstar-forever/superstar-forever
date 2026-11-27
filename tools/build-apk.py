#!/usr/bin/env python3
"""Build the patched, signed split APKs.

Usage: tools/build-apk.py --game <id> --server http://127.0.0.1:18384  (server on the host, see below)
       tools/build-apk.py --game <id> --embedded                        (server inside the APK)

  --server URL   where the game fetches its startup manifest (the local server). It must fit
                 in the length of the original URL (67 characters for SSOMG); it's padded with a filler
                 path the server ignores. For a server on the PC, use http://127.0.0.1:<port> (game.json
                 "port") with `adb reverse tcp:<port> tcp:<port>`: the same address as the server inside
                 the game, so the tables the game caches (they contain the server's address) stay valid
                 when switching between both kinds of builds. Start the server with the same
                 --public-url. Other examples: http://10.0.2.2:18384 (emulator -> host), http://192.168.1.10:18384.

Output: build/<game>/apk/base.apk and config.arm64_v8a.apk, aligned and signed with
keys/<game>.keystore (created on first run). Install both together:
  adb install-multiple build/<game>/apk/base.apk build/<game>/apk/config.arm64_v8a.apk

Changes made:
  base.apk    the file holding the manifest URL (game.json "manifestHeadFile": a KeyVault asset in
              SSOMG, global-metadata.dat in SSGF): every copy of the URL prefix ("manifestHead")
              -> --server (same length, so the file keeps its layout)
              AndroidManifest.xml: prompt/ad permissions renamed (DISABLED_PERMISSIONS)
  config.apk  lib/arm64-v8a/libil2cpp.so -> build/<game>/libil2cpp.so (tools/apply-patches.py)
"""

import argparse, os, shutil, subprocess, sys, zipfile
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from game import ROOT, select

GAME, _ = select()
SDK = Path(os.environ.get("ANDROID_HOME") or sys.exit("set ANDROID_HOME to the Android SDK"))
BUILD_TOOLS = sorted((SDK / "build-tools").iterdir())[-1]
OUT = GAME.build / "apk"
UNPACKED = GAME.unpacked / "apks"
KEYSTORE = GAME.keystore
KEY_PASS = f"{GAME.id}-offline"  # local signing key, not a secret
HEAD_FILE = GAME["manifestHeadFile"]
ORIGINAL_MANIFEST_HEAD = GAME["manifestHead"].encode()


def manifest_head(server: str) -> bytes:
    """Same-length replacement; the game appends 'real/manifest/<version>.txt'."""
    base = server.rstrip("/")
    room = len(ORIGINAL_MANIFEST_HEAD) - len(base) - 2  # two slashes around the filler
    if room < 1:
        sys.exit(f"--server too long: max {len(ORIGINAL_MANIFEST_HEAD) - 3} characters")
    head_b = f"{base}/{(f'{GAME.id}-offline-' * 8)[:room]}/".encode()
    assert len(head_b) == len(ORIGINAL_MANIFEST_HEAD), (head_b, len(head_b))
    return head_b


# Permissions that cause runtime prompts (or only serve ads/tracking). They are renamed in
# the binary AndroidManifest.xml to names Android doesn't know, so they're never granted
# and requests for them are denied without a prompt.
DISABLED_PERMISSIONS = [
    "android.permission.READ_PHONE_STATE",
    "android.permission.POST_NOTIFICATIONS",
    "android.permission.CAMERA",
    "android.permission.READ_MEDIA_IMAGES",
    "android.permission.READ_MEDIA_VIDEO",
    "android.permission.READ_MEDIA_AUDIO",
    "android.permission.READ_EXTERNAL_STORAGE",
    "android.permission.WRITE_EXTERNAL_STORAGE",
    "android.permission.ACCESS_ADSERVICES_AD_ID",
    "android.permission.ACCESS_ADSERVICES_ATTRIBUTION",
    "android.permission.ACCESS_ADSERVICES_TOPICS",
    "com.google.android.gms.permission.AD_ID",
]


def disable_permissions(axml: bytes) -> bytes:
    """Same-length rename inside the manifest's UTF-16 string pool: the leading 'android.'
    or 'com.' becomes 'disabled' / 'off.'. Matches whole pool entries only (length prefix
    + string + terminator), so substrings of other strings are untouched."""
    for perm in DISABLED_PERMISSIONS:
        old = perm.encode("utf-16-le")
        new_name = ("disabled" + perm[8:]) if perm.startswith("android.") else ("off." + perm[4:])
        new = new_name.encode("utf-16-le")
        assert len(new) == len(old)
        entry = len(perm).to_bytes(2, "little") + old + b"\0\0"
        n = axml.count(entry)
        if n == 0:
            continue  # this game doesn't request it
        if n > 1:
            sys.exit(f"permission {perm} found {n} times in AndroidManifest.xml")
        axml = axml.replace(entry, len(perm).to_bytes(2, "little") + new + b"\0\0")
    return axml


def repack(src: Path, dst: Path, replace: dict, add: dict = None):
    """Copy a zip entry by entry (same compression), replacing some files, adding new ones
    (stored uncompressed, as native libs must be) and dropping the old signature."""
    with zipfile.ZipFile(src) as zin, zipfile.ZipFile(dst, "w") as zout:
        for info in zin.infolist():
            name = info.filename
            if name.startswith("META-INF/") and name.rsplit(".", 1)[-1] in ("SF", "RSA", "DSA", "EC", "MF"):
                continue
            data = replace.pop(name) if name in replace else zin.read(name)
            new = zipfile.ZipInfo(name, date_time=info.date_time)
            new.compress_type = info.compress_type
            new.external_attr = info.external_attr
            zout.writestr(new, data)
        for name, data in (add or {}).items():
            new = zipfile.ZipInfo(name, date_time=(1981, 1, 1, 1, 1, 2))
            new.compress_type = zipfile.ZIP_STORED
            new.external_attr = 0o644 << 16
            zout.writestr(new, data)
    if replace:
        sys.exit(f"entries not found in {src.name}: {list(replace)}")


def redirect_libmain(libmain: bytes) -> bytes:
    """libmain.so (loaded first by Unity) depends on libdl.so; rename that dependency to
    libss.so (same length) so our server library loads at process start. libss.so itself
    depends on libdl.so, so libmain still gets its symbols."""
    old, new = b"\0libdl.so\0", b"\0libss.so\0"
    if libmain.count(old) != 1:
        sys.exit("libdl.so dependency not found exactly once in libmain.so")
    return libmain.replace(old, new)


def align_and_sign(unsigned: Path, final: Path):
    aligned = unsigned.with_suffix(".aligned.apk")
    # -P 16: page-align uncompressed .so files to 16 KB (works on 4 KB and 16 KB devices).
    subprocess.run([str(BUILD_TOOLS / "zipalign"), "-f", "-P", "16", "4", str(unsigned), str(aligned)], check=True)
    subprocess.run(
        [
            str(BUILD_TOOLS / "apksigner"),
            "sign",
            "--ks",
            str(KEYSTORE),
            "--ks-pass",
            f"pass:{KEY_PASS}",
            "--out",
            str(final),
            str(aligned),
        ],
        check=True,
    )
    unsigned.unlink()
    aligned.unlink()
    for idsig in final.parent.glob(final.name + ".idsig"):
        idsig.unlink()


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--server", help="manifest server URL (default with --embedded: http://127.0.0.1:<game.json port>)")
    ap.add_argument(
        "--embedded",
        action="store_true",
        help="ship the server inside the APK (build/<game>/libss.so from tools/build-android.sh)",
    )
    args = ap.parse_args()
    if args.embedded and not args.server:
        args.server = f"http://127.0.0.1:{GAME.cfg.get('port', 18384)}"
    if not args.server:
        ap.error("--server is required (or use --embedded)")

    OUT.mkdir(parents=True, exist_ok=True)
    if not KEYSTORE.exists():
        KEYSTORE.parent.mkdir(parents=True, exist_ok=True)
        subprocess.run(
            [
                "keytool",
                "-genkeypair",
                "-keystore",
                str(KEYSTORE),
                "-alias",
                GAME.id,
                "-storepass",
                KEY_PASS,
                "-keypass",
                KEY_PASS,
                "-keyalg",
                "RSA",
                "-keysize",
                "2048",
                "-validity",
                "36500",
                "-dname",
                f"CN={GAME.id.upper()} Offline",
            ],
            check=True,
        )

    # base.apk: manifest URL
    with zipfile.ZipFile(UNPACKED / "base.apk") as z:
        head_file = z.read(HEAD_FILE)
        manifest = z.read("AndroidManifest.xml")
    if ORIGINAL_MANIFEST_HEAD not in head_file:
        sys.exit(f"manifest URL not found in {HEAD_FILE}")
    head = manifest_head(args.server)
    print(f"manifest URL prefix: {head.decode()}")
    repack(
        UNPACKED / "base.apk",
        OUT / "base.unsigned.apk",
        {
            HEAD_FILE: head_file.replace(ORIGINAL_MANIFEST_HEAD, head),
            "AndroidManifest.xml": disable_permissions(manifest),
        },
    )
    align_and_sign(OUT / "base.unsigned.apk", OUT / "base.apk")

    # config.arm64_v8a.apk: patched libil2cpp.so
    subprocess.run([sys.executable, str(ROOT / "tools/apply-patches.py"), "--game", GAME.id], check=True)
    replace = {"lib/arm64-v8a/libil2cpp.so": (GAME.build / "libil2cpp.so").read_bytes()}
    add = {}
    if args.embedded:
        libss = GAME.build / "libss.so"
        if not libss.exists():
            sys.exit(f"{libss.relative_to(ROOT)} missing: run tools/build-android.sh {GAME.id} first")
        with zipfile.ZipFile(UNPACKED / "config.arm64_v8a.apk") as z:
            replace["lib/arm64-v8a/libmain.so"] = redirect_libmain(z.read("lib/arm64-v8a/libmain.so"))
        add["lib/arm64-v8a/libss.so"] = libss.read_bytes()
        print("embedded server: libss.so added, libmain.so -> libss.so")
    repack(UNPACKED / "config.arm64_v8a.apk", OUT / "config.unsigned.apk", replace, add)
    align_and_sign(OUT / "config.unsigned.apk", OUT / "config.arm64_v8a.apk")

    print(
        f"\nInstall: adb install-multiple {OUT.relative_to(ROOT)}/base.apk {OUT.relative_to(ROOT)}/config.arm64_v8a.apk"
    )


if __name__ == "__main__":
    main()
