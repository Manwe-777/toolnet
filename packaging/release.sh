#!/usr/bin/env bash
# Publishes a Linux release on GitHub: static binaries, SHA256SUMS,
# install.sh (the one-liner fetches releases/latest/download/install.sh) and
# VERSION (what `toolnet update` compares with).
#
#   packaging/release.sh 0.3.0                builds (packaging/build-linux.sh), then publishes v0.3.0
#   SKIP_BUILD=1 packaging/release.sh 0.3.0   reuse dist/linux
#
# VERSION must be the crates' version (daemon/ and tray/Cargo.toml): bump them first.
set -euo pipefail
cd "$(dirname "$0")/.."
VERSION=${1:?usage: release.sh VERSION}
OUT=dist/linux
for c in daemon tray; do
  v=$(sed -n 's/^version = "\(.*\)"/\1/p' $c/Cargo.toml | head -1)
  [ "$v" = "$VERSION" ] || { echo "$c/Cargo.toml is $v, not $VERSION: bump it first" >&2; exit 1; }
done
[ -n "${SKIP_BUILD:-}" ] || packaging/build-linux.sh
cp packaging/install.sh "$OUT/install.sh"
echo "$VERSION" > "$OUT/VERSION"
(cd "$OUT" && sha256sum toolnet-linux-* toolnet-tray-linux-* > SHA256SUMS && cat SHA256SUMS)
gh release create "v$VERSION" --title "toolnet $VERSION" --notes "Linux: \`curl -fsSL https://github.com/Manwe-777/toolnet/releases/latest/download/install.sh | sh -s -- --owner npub1... --net home\`" \
  "$OUT"/toolnet-linux-* "$OUT"/toolnet-tray-linux-* "$OUT/SHA256SUMS" "$OUT/install.sh" "$OUT/VERSION"
