#!/usr/bin/env bash
# Unpack a game's original .xapk or .apk into work/<game>/unpacked/.
# Usage: tools/unpack.sh <game> <file.xapk|file.apk>     (e.g. a file in original/; safe to re-run)
set -euo pipefail
source "$(dirname "$0")/game.sh"
[ $# -ge 1 ] && [ -f "$1" ] || { echo "usage: $0 $GAME <file.xapk|file.apk>" >&2; exit 2; }
SRC="$1"
OUT="$WORK/unpacked"
rm -rf "$OUT/apks" "$OUT/base" "$OUT/arm64"
mkdir -p "$OUT/apks" "$OUT/base" "$OUT/arm64"
case "$SRC" in
  *.xapk)
    unzip -oq "$SRC" -d "$OUT/apks"
    mv "$OUT/apks/$PACKAGE.apk" "$OUT/apks/base.apk"
    unzip -oq "$OUT/apks/config.arm64_v8a.apk" -d "$OUT/arm64" ;;
  *.apk)  # one APK with all ABIs: the arm64 libraries come from it too
    cp "$SRC" "$OUT/apks/base.apk"
    unzip -oq "$SRC" 'lib/arm64-v8a/*' -d "$OUT/arm64" ;;
  *) echo "not an .xapk or .apk: $SRC" >&2; exit 1 ;;
esac
unzip -oq "$OUT/apks/base.apk" -d "$OUT/base"
echo "Unpacked $(basename "$SRC") to work/$GAME/unpacked"
