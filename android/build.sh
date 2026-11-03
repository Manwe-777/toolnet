#!/usr/bin/env bash
# Builds the Rust core for the phone, then the APK; with --install, installs
# it over adb (wireless debugging works: `adb devices` must list the phone).
#   android/build.sh [--install]
set -euo pipefail
cd "$(dirname "$0")"
export ANDROID_NDK_HOME=${ANDROID_NDK_HOME:-$(ls -d ~/Android/Sdk/ndk/* | sort -V | tail -1)}
rm -rf app/src/main/jniLibs
cargo ndk -t arm64-v8a --platform 26 -o app/src/main/jniLibs build --profile android -p toolnet-android
# cargo-ndk copies every .so it finds; the app only loads the core
find app/src/main/jniLibs -name '*.so' ! -name libtoolnet_android.so -delete
mise exec -- ./gradlew -q assembleDebug
APK=app/build/outputs/apk/debug/app-debug.apk
ls -la "$APK"
if [ "${1:-}" = --install ]; then
  ~/Android/Sdk/platform-tools/adb install -r "$APK"
fi
