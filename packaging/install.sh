#!/bin/sh
# toolnet installer for Linux. Downloads the static binaries for this machine,
# checks them against SHA256SUMS, then runs `sudo toolnet install` with the
# same arguments:
#
#   curl -fsSL https://github.com/Manwe-777/toolnet/releases/latest/download/install.sh \
#     | sh -s -- --owner npub1... --net home
#
# Run it again to update (the device keeps its identity; --owner can be left
# out). Every option is passed to `toolnet install` (see `toolnet install
# --help`): --args "--advertise-exit-node", --operator USER, --no-tray, ...
#
# TOOLNET_DOWNLOAD overrides where the files come from (a release URL or a
# local directory, for testing).
set -eu

BASE=${TOOLNET_DOWNLOAD:-https://github.com/Manwe-777/toolnet/releases/latest/download}

fail() { echo "toolnet install: $*" >&2; exit 1; }

[ "$(uname -s)" = Linux ] || fail "only Linux for now (macOS and Windows are planned)"
case $(uname -m) in
  x86_64 | amd64) ARCH=x86_64 ;;
  aarch64 | arm64) ARCH=aarch64 ;;
  *) fail "no build for $(uname -m) yet (x86_64 and aarch64 only)" ;;
esac

if command -v curl >/dev/null 2>&1; then
  fetch() { curl -fsSL --retry 3 -o "$2" "$1"; }
elif command -v wget >/dev/null 2>&1; then
  fetch() { wget -q -O "$2" "$1"; }
else
  fail "needs curl or wget"
fi
case $BASE in
  /*) fetch() { cp "$1" "$2"; } ;;
esac
if command -v sha256sum >/dev/null 2>&1; then
  sha256() { sha256sum "$1" | cut -d' ' -f1; }
elif command -v shasum >/dev/null 2>&1; then
  sha256() { shasum -a 256 "$1" | cut -d' ' -f1; }
else
  fail "needs sha256sum"
fi

# The tray icon only where there's a desktop
TRAY=1
for a in "$@"; do [ "$a" = --no-tray ] && TRAY=; done
[ -n "${WAYLAND_DISPLAY:-}${DISPLAY:-}" ] || TRAY=

TMP=$(mktemp -d)
trap 'rm -rf "$TMP"' EXIT INT TERM

fetch "$BASE/SHA256SUMS" "$TMP/SHA256SUMS" || fail "can't download $BASE/SHA256SUMS"
get() {
  echo "downloading $1-linux-$ARCH"
  fetch "$BASE/$1-linux-$ARCH" "$TMP/$1" || fail "can't download $BASE/$1-linux-$ARCH"
  want=$(awk -v f="$1-linux-$ARCH" '$2 == f || $2 == "*" f { print $1 }' "$TMP/SHA256SUMS")
  [ -n "$want" ] || fail "$1-linux-$ARCH isn't listed in SHA256SUMS"
  [ "$(sha256 "$TMP/$1")" = "$want" ] || fail "$1-linux-$ARCH doesn't match its checksum: not installing"
  chmod 755 "$TMP/$1"
}
get toolnet
[ -n "$TRAY" ] && get toolnet-tray

if [ "$(id -u)" = 0 ]; then
  "$TMP/toolnet" install "$@"
elif command -v sudo >/dev/null 2>&1; then
  sudo "$TMP/toolnet" install "$@"
else
  fail "run as root, or install sudo"
fi
