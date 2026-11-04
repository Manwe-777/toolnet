#!/usr/bin/env bash
# The owner app's download card data for an APK:
#   packaging/apk-info.sh android/dist/toolnet.apk 0.4.0 > version.json
set -euo pipefail
APK=${1:?usage: apk-info.sh APK VERSION}
VERSION=${2:?usage: apk-info.sh APK VERSION}
# The version inside the APK has to be the one we announce
if command -v aapt2 >/dev/null || ls ~/Android/Sdk/build-tools/*/aapt2 >/dev/null 2>&1; then
  AAPT=$(command -v aapt2 || ls ~/Android/Sdk/build-tools/*/aapt2 | sort -V | tail -1)
  got=$("$AAPT" dump badging "$APK" | sed -n "s/.*versionName='\([^']*\)'.*/\1/p")
  [ "$got" = "$VERSION" ] || { echo "$APK is version $got, not $VERSION" >&2; exit 1; }
fi
printf '{"version":"%s","size":%s,"sha256":"%s","published":"%s"}\n' \
  "$VERSION" "$(stat -c %s "$APK")" "$(sha256sum "$APK" | cut -d' ' -f1)" "$(date -u +%FT%TZ)"
