#!/usr/bin/env bash
# Cross-compile the server for Android arm64 -> build/<game>/libss.so (loaded by
# libmain.so, see server/src/android.rs and tools/build-apk.py --embedded).
# Usage: tools/build-android.sh <game>      (the library is the same for every game)
set -euo pipefail
source "$(dirname "$0")/game.sh"
SDK="${ANDROID_HOME:?set ANDROID_HOME to the Android SDK}"
NDK="$(ls -d "$SDK"/ndk/* | sort -V | tail -1)"
BIN="$NDK/toolchains/llvm/prebuilt/linux-x86_64/bin"
export CARGO_TARGET_AARCH64_LINUX_ANDROID_LINKER="$BIN/aarch64-linux-android24-clang"
export CC_aarch64_linux_android="$BIN/aarch64-linux-android24-clang"
export AR_aarch64_linux_android="$BIN/llvm-ar"
# 16 KB pages (future devices); keep libdl/libm as real dependencies because libmain.so's
# own libdl.so dependency is redirected to this library.
export CARGO_TARGET_AARCH64_LINUX_ANDROID_RUSTFLAGS="-C link-arg=-Wl,-z,max-page-size=16384 -C link-arg=-Wl,-soname,libss.so -C link-arg=-Wl,--no-as-needed -C link-arg=-ldl -C link-arg=-lm"
cd "$ROOT/server"
export CARGO_HOME="${CARGO_HOME:-$ROOT/server/.cargo-home}"
cargo build --release --offline --target aarch64-linux-android --lib
mkdir -p "$BUILD"
cp target/aarch64-linux-android/release/libsuperstar_server.so "$BUILD/libss.so"
"$BIN/llvm-readelf" -d "$BUILD/libss.so" | grep NEEDED
ls -la "$BUILD/libss.so"
