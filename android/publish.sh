#!/usr/bin/env bash
# Builds the signed release APK and publishes it next to the owner app on
# the homelab (the toolnet-app container serves it at /download/toolnet.apk).
#   android/publish.sh [VERSION]      e.g. 0.2.0 (default: the build's)
#   TOOLNET_PUBLISH=user@host:/dir    where to copy (default: the homelab)
set -euo pipefail
cd "$(dirname "$0")"
VERSION=${1:-0.1.0}
DEST=${TOOLNET_PUBLISH:-server.local:/home/manuel/appdata/toolnet-app/download}
./build.sh --release -PappVersion="$VERSION" >/dev/null
APK=dist/toolnet.apk
SHA=$(sha256sum "$APK" | cut -d' ' -f1)
SIZE=$(stat -c %s "$APK")
printf '{"version":"%s","size":%s,"sha256":"%s","published":"%s"}\n' "$VERSION" "$SIZE" "$SHA" "$(date -u +%FT%TZ)" > dist/version.json
host=${DEST%%:*} dir=${DEST#*:}
ssh "$host" "mkdir -p '$dir'"
scp -q "$APK" dist/version.json "$DEST/"
echo "published toolnet $VERSION ($SIZE bytes, sha256 $SHA) to $DEST"
