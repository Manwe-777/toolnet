#!/usr/bin/env bash
# Publishes a Linux release on GitHub: static binaries, SHA256SUMS and
# install.sh, which the one-liner fetches from releases/latest/download/.
#
#   packaging/release.sh 0.2.0          builds (packaging/build-linux.sh), then publishes v0.2.0
#   SKIP_BUILD=1 packaging/release.sh   reuse dist/linux
#
# The downloads are only public once the repo is.
set -euo pipefail
cd "$(dirname "$0")/.."
VERSION=${1:?usage: release.sh VERSION}
OUT=dist/linux
[ -n "${SKIP_BUILD:-}" ] || packaging/build-linux.sh
cp packaging/install.sh "$OUT/install.sh"
(cd "$OUT" && sha256sum toolnet-linux-* toolnet-tray-linux-* > SHA256SUMS && cat SHA256SUMS)
gh release create "v$VERSION" --title "toolnet $VERSION" --notes "Linux: \`curl -fsSL https://github.com/Manwe-777/toolnet/releases/latest/download/install.sh | sh -s -- --owner npub1... --net home\`" \
  "$OUT"/toolnet-linux-* "$OUT"/toolnet-tray-linux-* "$OUT/SHA256SUMS" "$OUT/install.sh"
