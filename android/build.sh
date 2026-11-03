#!/usr/bin/env bash
# Builds the Rust core for the phone, then the APK; with --install, installs
# it over adb (wireless debugging works: `adb devices` must list the phone).
#   android/build.sh [--install]
set -euo pipefail
cd "$(dirname "$0")"
export ANDROID_NDK_HOME=${ANDROID_NDK_HOME:-$(ls -d ~/Android/Sdk/ndk/* | sort -V | tail -1)}
cargo ndk -t arm64-v8a --platform 26 -o app/src/main/jniLibs build --release -p toolnet-android
mise exec -- ./gradlew -q assembleDebug
APK=app/build/outputs/apk/debug/app-debug.apk
ls -la "$APK"
if [ "${1:-}" = --install ]; then
  ~/Android/Sdk/platform-tools/adb install -r "$APK"
fi
