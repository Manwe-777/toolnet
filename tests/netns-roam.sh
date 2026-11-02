#!/usr/bin/env bash
# A network switch, in namespaces: alpha's address changes under a running
# daemon, which kills its websockets to the Nostr relay without closing them.
# Checks that alpha notices, reconnects at once (not after a minute-long
# timeout), republishes its new address, and still follows the owner.
#
#   outer ns: bridge 10.9.0.254, ToolDB relay, owner commands
#   alpha:    10.9.0.1, then 10.9.0.11
#   beta:     10.9.0.2
set -euo pipefail
HERE=$(cd "$(dirname "$0")/.." && pwd)
BIN=${BIN:-$HERE/target/debug/toolnet}
W=${WORK:-$(mktemp -d)}
mkdir -p "$W"

exec unshare -Urn --kill-child bash -s "$BIN" "$W" "$HERE" <<'INNER'
set -euo pipefail
BIN=$1 W=$2 HERE=$3
RELAY=ws://10.9.0.254:7791
pids=()
trap 'kill "${pids[@]}" 2>/dev/null || true' EXIT
plain() { sed "s/\x1b\[[0-9;]*m//g" "$1"; }
wait_for() { for _ in $(seq "$3"); do plain "$1" | grep -qE "$2" && return 0; sleep 1; done; echo "TIMEOUT /$2/ in $1"; plain "$1" | tail -15; exit 1; }
inns() { local ns=$1; shift; nsenter -t "${NS[$ns]}" -n "$@"; }
declare -A NS

ip link set lo up
ip link add br0 type bridge && ip addr add 10.9.0.254/24 dev br0 && ip link set br0 up
for n in alpha beta; do unshare -n sleep infinity & NS[$n]=$!; pids+=($!); done
sleep 0.3
i=1
for n in alpha beta; do
  ip link add "v-$n" type veth peer name eth0 netns "${NS[$n]}"
  ip link set "v-$n" master br0 up
  inns "$n" sh -c "ip link set lo up; ip addr add 10.9.0.$i/24 dev eth0; ip link set eth0 up"
  i=$((i+1))
done

node "$HERE/tests/local-relay.mjs" 7791 10.9.0.254 > "$W/relay.log" 2>&1 & pids+=($!)
wait_for "$W/relay.log" listening 10

echo "=== owner: network + approvals"
"$BIN" --state "$W/owner" owner create --net home --relay "$RELAY" > /dev/null
OWNER=$(python3 -c "import json;print(json.load(open('$W/owner/config.json'))['owner'])")
for n in alpha beta; do
  "$BIN" --state "$W/$n" init --owner "$OWNER" --net home --relay "$RELAY" > /dev/null
done
for n in alpha beta; do
  "$BIN" --state "$W/owner" owner approve "$("$BIN" --state "$W/$n" id | awk '/^hex/{print $2}')" --name "$n"
done

for n in alpha beta; do
  inns $n env RUST_LOG=${ROAM_LOG:-info} "$BIN" --state "$W/$n" up --no-tun --offline > "$W/$n.log" 2>&1 & pids+=($!)
done
wait_for "$W/alpha.log" "connected peer=beta" 30
wait_for "$W/alpha.log" 'published node info addrs=\["10.9.0.1:' 20

echo "=== alpha switches networks: 10.9.0.1 -> 10.9.0.11"
inns alpha sh -c "ip addr add 10.9.0.11/24 dev eth0; ip addr del 10.9.0.1/24 dev eth0"
t0=$(date +%s)
wait_for "$W/alpha.log" "network changed" 20
wait_for "$W/alpha.log" "Nostr relays reconnected" 20
wait_for "$W/alpha.log" 'published node info addrs=\["10.9.0.11:' 20
echo "  reconnected and republished after $(( $(date +%s) - t0 )) s"

echo "=== the owner removes beta after the switch: alpha must follow"
"$BIN" --state "$W/owner" owner remove beta
wait_for "$W/alpha.log" "peer removed" 20
echo "  alpha dropped beta"
echo PASS
INNER
