#!/usr/bin/env bash
# Builds static (musl) toolnet + toolnet-tray for x86_64 and aarch64, in
# Docker with cargo-zigbuild, so they run on any Linux distro:
#
#   packaging/build-linux.sh            -> dist/linux/toolnet-linux-{x86_64,aarch64}, ...
#
# The cargo registry and target dir are cached in Docker volumes.
set -euo pipefail
cd "$(dirname "$0")/.."
IMAGE=${ZIGBUILD_IMAGE:-ghcr.io/rust-cross/cargo-zigbuild:0.24.0}
# The image's rustc is older than our dependencies need
TOOLCHAIN=${TOOLCHAIN:-1.99.1}
OUT=dist/linux
mkdir -p "$OUT"
docker run --rm \
  -v "$PWD":/src -w /src \
  -v toolnet-cargo-registry:/usr/local/cargo/registry \
  -v toolnet-musl-target:/target \
  -v toolnet-rustup:/usr/local/rustup \
  -e CARGO_TARGET_DIR=/target -e RUSTUP_TOOLCHAIN="$TOOLCHAIN" \
  "$IMAGE" sh -c '
    set -e
    rustup toolchain install "$RUSTUP_TOOLCHAIN" --profile minimal \
      -t x86_64-unknown-linux-musl,aarch64-unknown-linux-musl >/dev/null
    for t in x86_64-unknown-linux-musl aarch64-unknown-linux-musl; do
      cargo zigbuild --locked --release --target $t -p toolnet -p toolnet-tray
    done'
for arch in x86_64 aarch64; do
  for bin in toolnet toolnet-tray; do
    docker run --rm -v toolnet-musl-target:/target -v "$PWD/$OUT":/out alpine \
      cp "/target/$arch-unknown-linux-musl/release/$bin" "/out/$bin-linux-$arch"
  done
done
docker run --rm -v "$PWD/$OUT":/out alpine chown "$(id -u):$(id -g)" /out -R
ls -l "$OUT"
