#!/usr/bin/env bash
# The Linux installer on clean distros, in Docker containers running systemd:
# install.sh from a local "release" directory, the device joins a throwaway
# network (owner on this machine, public Nostr relays), re-running install.sh
# and `toolnet update` keep the identity and the operator, a tampered download
# is refused, uninstall keeps the identity, --purge deletes it.
#
#   packaging/build-linux.sh && tests/installer.sh [debian fedora arch]
#
# Needs Docker (unprivileged: see the docker run below). Images: tests/installer/<distro>.Dockerfile
set -euo pipefail
cd "$(dirname "$0")/.."
DISTROS=${*:-debian fedora arch}
HOST_BIN=${HOST_BIN:-target/release/toolnet}
WORK=$(mktemp -d)
trap 'docker rm -f $(docker ps -aq --filter label=toolnet-installer-test) >/dev/null 2>&1 || true; rm -rf "$WORK"' EXIT

# The "release": what packaging/release.sh would upload
DL=$WORK/dl
mkdir -p "$DL"
cp dist/linux/toolnet-linux-x86_64 dist/linux/toolnet-tray-linux-x86_64 packaging/install.sh "$DL/"
(cd "$DL" && sha256sum toolnet-linux-* toolnet-tray-linux-* > SHA256SUMS)
CURRENT=$(sed -n 's/^version = "\(.*\)"/\1/p' daemon/Cargo.toml | head -1)
echo "$CURRENT" > "$DL/VERSION"
# A "newer" release with the same binaries, for `toolnet update`
DL2=$WORK/dl2
cp -r "$DL" "$DL2"
echo 99.0.0 > "$DL2/VERSION"

# A throwaway network, owned from this machine
OWNER_STATE=$WORK/owner
export TOOLNET_STATE=$OWNER_STATE
NET=it$RANDOM
$HOST_BIN owner create --net "$NET" >/dev/null
OWNER=$(python3 -c 'import json,sys; print(json.load(open(sys.argv[1]))["owner"])' "$OWNER_STATE/config.json")
echo "network $NET, owner $OWNER"

pass=0
fail() { echo "FAIL [$distro]: $*"; docker exec "$c" journalctl -u toolnet --no-pager -n 30 || true; exit 1; }
for distro in $DISTROS; do
  docker build -q -t "tn-$distro" -f "tests/installer/$distro.Dockerfile" tests/installer >/dev/null
  # Not --privileged: systemd in a privileged container takes over the host's
  # VTs and kills the desktop session. SYS_ADMIN is for remounting the
  # (private) cgroup tree rw in boot.sh; NET_ADMIN + /dev/net/tun for the TUN
  c=$(docker run -d --cgroupns=private --cap-add SYS_ADMIN --cap-add NET_ADMIN --device /dev/net/tun \
        --tmpfs /run --tmpfs /run/lock \
        --label toolnet-installer-test -v "$DL":/dl:ro -v "$DL2":/dl2:ro --hostname "tn-$distro" "tn-$distro")
  docker exec "$c" sh -c 'until systemctl is-system-running 2>/dev/null | grep -qE "running|degraded"; do sleep 1; done'
  docker exec "$c" useradd -m alice
  echo "== $distro: install"
  docker exec -e TOOLNET_DOWNLOAD=/dl "$c" sh /dl/install.sh --owner "$OWNER" --net "$NET" --operator alice \
    > "$WORK/$distro.log" 2>&1 || { cat "$WORK/$distro.log"; fail "install.sh"; }
  grep -q "toolnet:join?device=npub1" "$WORK/$distro.log" || fail "no join QR printed"
  docker exec "$c" systemctl is-active -q toolnet || fail "service not active"
  docker exec "$c" sh -c 'command -v ip && command -v nft' >/dev/null || fail "iproute2/nftables not installed"
  docker exec "$c" grep -q alice /etc/systemd/system/toolnet.service.d/operator.conf || fail "operator not set"
  [ "$(docker exec "$c" stat -c %a /var/lib/toolnet)" = 700 ] || fail "state dir not 0700"
  DEVICE=$(docker exec "$c" toolnet --state /var/lib/toolnet id | awk '/^device:/ {print $2}')

  echo "== $distro: approve $DEVICE"
  $HOST_BIN owner approve "$DEVICE" --name "$distro" >/dev/null
  for i in $(seq 60); do
    st=$(docker exec "$c" toolnet status --json 2>/dev/null | python3 -c 'import json,sys; s=json.load(sys.stdin); print(s["state"], s.get("ip") or "")' || true)
    case $st in member*) break ;; esac
    sleep 2
  done
  case $st in member*) echo "   joined: $st" ;; *) fail "not a member after 2 min (state: $st)" ;; esac

  echo "== $distro: update keeps the identity"
  docker exec -e TOOLNET_DOWNLOAD=/dl "$c" sh /dl/install.sh > "$WORK/$distro-2.log" 2>&1 || { cat "$WORK/$distro-2.log"; fail "update"; }
  grep -q "toolnet:join" "$WORK/$distro-2.log" && fail "update printed a new join QR"
  [ "$(docker exec "$c" toolnet --state /var/lib/toolnet id | awk '/^device:/ {print $2}')" = "$DEVICE" ] || fail "identity changed"
  docker exec "$c" systemctl is-active -q toolnet || fail "service not active after update"

  echo "== $distro: toolnet update"
  docker exec -e TOOLNET_DOWNLOAD=/dl "$c" toolnet update --check | grep -q "is the latest" || fail "update --check: not the latest"
  docker exec -e TOOLNET_DOWNLOAD=/dl "$c" toolnet update | grep -q "is the latest" || fail "update: should be a no-op"
  docker exec -e TOOLNET_DOWNLOAD=/dl2 "$c" toolnet update --check | grep -q "99.0.0 is available" || fail "update --check: missed 99.0.0"
  docker exec -e TOOLNET_DOWNLOAD=/dl2 "$c" toolnet update > "$WORK/$distro-3.log" 2>&1 || { cat "$WORK/$distro-3.log"; fail "update"; }
  grep -q "updated toolnet $CURRENT -> 99.0.0" "$WORK/$distro-3.log" || { cat "$WORK/$distro-3.log"; fail "update didn't say it updated"; }
  [ "$(docker exec "$c" toolnet --state /var/lib/toolnet id | awk '/^device:/ {print $2}')" = "$DEVICE" ] || fail "identity changed on update"
  docker exec "$c" grep -q alice /etc/systemd/system/toolnet.service.d/operator.conf || fail "update lost the operator"
  docker exec "$c" systemctl is-active -q toolnet || fail "service not active after toolnet update"
  # A tampered binary is refused
  docker exec "$c" sh -c 'mkdir /bad && cp /dl2/* /bad/ && echo x >> /bad/toolnet-linux-x86_64'
  docker exec -e TOOLNET_DOWNLOAD=/bad "$c" toolnet update > "$WORK/$distro-4.log" 2>&1 && fail "tampered binary installed"
  grep -q "doesn't match its checksum" "$WORK/$distro-4.log" || { cat "$WORK/$distro-4.log"; fail "tampered binary: wrong error"; }
  docker exec "$c" systemctl is-active -q toolnet || fail "service not active after a refused update"

  echo "== $distro: uninstall"
  docker exec "$c" toolnet uninstall >/dev/null || fail "uninstall"
  docker exec "$c" sh -c '! test -e /usr/local/bin/toolnet && ! test -e /etc/systemd/system/toolnet.service && test -e /var/lib/toolnet/device.key' \
    || fail "uninstall left files / lost the identity"
  docker exec -e TOOLNET_DOWNLOAD=/dl "$c" sh /dl/install.sh --no-start >/dev/null 2>&1 || fail "reinstall"
  docker exec "$c" toolnet uninstall --purge >/dev/null || fail "uninstall --purge"
  docker exec "$c" test ! -e /var/lib/toolnet || fail "--purge kept the state"

  $HOST_BIN owner remove "$distro" >/dev/null
  docker rm -f "$c" >/dev/null
  echo "== $distro: ok"
  pass=$((pass + 1))
done
echo "installer: $pass distro(s) ok"
