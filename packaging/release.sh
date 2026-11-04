#!/usr/bin/env bash
# Publishes a release on GitHub, Linux and Android at the same version:
#   - static binaries, SHA256SUMS and install.sh (the one-liner fetches
#     releases/latest/download/install.sh)
#   - VERSION, what `toolnet update` compares with
#   - toolnet.apk + version.json, the signed Android app (same Rust core),
#     which the owner app's download card serves
# then redeploys the owner app so it offers the new APK.
#
#   packaging/release.sh 0.4.0                builds everything, then publishes v0.4.0
#   SKIP_BUILD=1 packaging/release.sh 0.4.0   reuse dist/linux and android/dist
#
# VERSION must be the crates' version (daemon/ and tray/Cargo.toml): bump them
# first. The APK is built with that version (-PappVersion).
set -euo pipefail
cd "$(dirname "$0")/.."
VERSION=${1:?usage: release.sh VERSION}
OUT=dist/linux
APK=android/dist/toolnet.apk
for c in daemon tray; do
  v=$(sed -n 's/^version = "\(.*\)"/\1/p' $c/Cargo.toml | head -1)
  [ "$v" = "$VERSION" ] || { echo "$c/Cargo.toml is $v, not $VERSION: bump it first" >&2; exit 1; }
done
if [ -z "${SKIP_BUILD:-}" ]; then
  packaging/build-linux.sh
  android/build.sh --release -PappVersion="$VERSION" >/dev/null
fi

cp packaging/install.sh "$OUT/install.sh"
echo "$VERSION" > "$OUT/VERSION"
(cd "$OUT" && sha256sum toolnet-linux-* toolnet-tray-linux-* > SHA256SUMS && cat SHA256SUMS)
packaging/apk-info.sh "$APK" "$VERSION" > android/dist/version.json

gh release create "v$VERSION" --title "toolnet $VERSION" --notes "$(cat <<EOF
**Linux:** \`curl -fsSL https://github.com/Manwe-777/toolnet/releases/latest/download/install.sh | sh -s -- --owner npub1... --net home\`
(already installed: \`sudo toolnet update\`)

**Android:** download from the owner app (https://manwe-777.github.io/toolnet/), or \`toolnet.apk\` below.
EOF
)" \
  "$OUT"/toolnet-linux-* "$OUT"/toolnet-tray-linux-* "$OUT/SHA256SUMS" "$OUT/install.sh" "$OUT/VERSION" \
  "$APK" android/dist/version.json
gh workflow run owner-app.yml --ref main   # the owner app picks up the new APK
