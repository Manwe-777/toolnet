#!/usr/bin/env bash
# Installs (or updates) toolnet as a systemd service. Run as root:
#
#   sudo packaging/install.sh BINARY [--state-from DIR] [--args "UP FLAGS"]
#
#   BINARY        the toolnet binary to install (target/release/toolnet)
#   --state-from  first install: copy this device's identity (config.json,
#                 device.key, endpoint.key) from a state dir made with
#                 `toolnet init`. Never copies owner.key.
#   --args        flags for `toolnet up`, e.g. "--exit-node homelab" or
#                 "--advertise-routes 192.168.1.0/24 --advertise-exit-node"
#   --log         RUST_LOG filter, e.g. "info,toolnet=debug" (default info)
#
# Updating is the same command without --state-from: it replaces the binary
# and restarts the service; the state in /var/lib/toolnet is kept.
set -euo pipefail
[ "$(id -u)" = 0 ] || { echo "run as root (sudo)"; exit 1; }
HERE=$(cd "$(dirname "$0")" && pwd)
BIN=${1:?usage: install.sh BINARY [--state-from DIR] [--args FLAGS] [--log FILTER]}; shift
FROM="" ARGS="" LOG=""
while [ $# -gt 0 ]; do
  case $1 in
    --state-from) FROM=$2; shift 2 ;;
    --args) ARGS=$2; shift 2 ;;
    --log) LOG=$2; shift 2 ;;
    *) echo "unknown option $1"; exit 1 ;;
  esac
done
STATE=/var/lib/toolnet

install -m 755 "$BIN" /usr/local/bin/toolnet
install -m 644 "$HERE/toolnet.service" /etc/systemd/system/toolnet.service
install -d -m 700 "$STATE"
if [ -n "$FROM" ]; then
  if [ -e "$STATE/device.key" ]; then
    echo "$STATE already has a device identity: not overwriting it"; exit 1
  fi
  for f in config.json device.key endpoint.key; do
    install -m 600 "$FROM/$f" "$STATE/$f"
  done
  echo "device identity copied from $FROM"
fi
D=/etc/systemd/system/toolnet.service.d
install -d "$D"
if [ -n "$ARGS" ] || [ ! -e "$D/args.conf" ]; then
  printf '[Service]\nEnvironment="TOOLNET_ARGS=%s"\n' "$ARGS" > "$D/args.conf"
fi
if [ -n "$LOG" ]; then
  printf '[Service]\nEnvironment="RUST_LOG=%s"\n' "$LOG" > "$D/log.conf"
fi
systemctl daemon-reload
systemctl enable toolnet > /dev/null 2>&1
systemctl restart toolnet
sleep 3
systemctl --no-pager --lines=0 status toolnet | head -4
echo "logs: journalctl -u toolnet -f"
