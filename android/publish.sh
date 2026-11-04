#!/usr/bin/env bash
# Builds the signed release APK and publishes it as a GitHub release
# (android-vVERSION, not marked latest: that's the Linux one-liner's). The
# owner app's Pages deploy picks up the newest one and serves it at
# ./download/toolnet.apk, with ./download/version.json.
#   android/publish.sh VERSION            e.g. 0.1.2
#   SKIP_BUILD=1 android/publish.sh 0.1.1 reuse dist/toolnet.apk
#   TOOLNET_PUBLISH=user@host:/dir        also copy there (a self-hosted owner app)
set -euo pipefail
cd "$(dirname "$0")"
VERSION=${1:?usage: publish.sh VERSION}
[ -n "${SKIP_BUILD:-}" ] || ./build.sh --release -PappVersion="$VERSION" >/dev/null
APK=dist/toolnet.apk
SHA=$(sha256sum "$APK" | cut -d' ' -f1)
SIZE=$(stat -c %s "$APK")
printf '{"version":"%s","size":%s,"sha256":"%s","published":"%s"}\n' "$VERSION" "$SIZE" "$SHA" "$(date -u +%FT%TZ)" > dist/version.json
gh release create "android-v$VERSION" --latest=false --title "toolnet Android $VERSION" \
  --notes "Install from the owner app (https://manwe-777.github.io/toolnet/), or download toolnet.apk. sha256 $SHA" \
  "$APK" dist/version.json
gh workflow run owner-app.yml --ref main   # redeploy the owner app with it
if [ -n "${TOOLNET_PUBLISH:-}" ]; then
  host=${TOOLNET_PUBLISH%%:*} dir=${TOOLNET_PUBLISH#*:}
  ssh "$host" "mkdir -p '$dir'"
  scp -q "$APK" dist/version.json "$TOOLNET_PUBLISH/"
fi
echo "published toolnet Android $VERSION ($SIZE bytes, sha256 $SHA)"
