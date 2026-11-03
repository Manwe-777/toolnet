#!/usr/bin/env bash
# Exit node, in network namespaces (unprivileged, offline).
#
#   outer ns: bridge 10.9.0.254 (a "home LAN" with no internet), ToolDB relay
#   alpha:    10.9.0.1, default route via 10.9.0.254 (goes nowhere)
#   exit:     10.9.0.2, plus a link to the "internet"
#   internet: 203.0.113.10, hosting 198.51.100.7
#
# alpha reaches 198.51.100.7 only through the exit node, only once the owner
# allows 0.0.0.0/0 for it, while its own peer and control traffic keeps
# flowing outside the tunnel.
set -euo pipefail
HERE=$(cd "$(dirname "$0")/.." && pwd)
BIN=${BIN:-$HERE/target/debug/toolnet}
W=${WORK:-$(mktemp -d)}
mkdir -p "$W"

exec unshare -Urn --kill-child bash -s "$BIN" "$W" "$HERE" <<'INNER'
set -euo pipefail
BIN=$1 W=$2 HERE=$3
RELAY=ws://10.9.0.254:7791
TARGET=198.51.100.7
pids=()
trap 'kill "${pids[@]}" 2>/dev/null || true' EXIT
plain() { sed "s/\x1b\[[0-9;]*m//g" "$1"; }
wait_for() { for _ in $(seq "$3"); do plain "$1" | grep -qE "$2" && return 0; sleep 1; done; echo "TIMEOUT /$2/ in $1"; plain "$1" | tail -15; exit 1; }
inns() { local ns=$1; shift; nsenter -t "${NS[$ns]}" -n "$@"; }
reach() { inns alpha ping -c 2 -i 0.3 -W 2 "$1" > /dev/null 2>&1; }
declare -A NS

ip link set lo up
ip link add br0 type bridge && ip addr add 10.9.0.254/24 dev br0 && ip link set br0 up
for n in alpha exit internet; do unshare -n sleep infinity & NS[$n]=$!; pids+=($!); done
sleep 0.3
i=1
for n in alpha exit; do
  ip link add "v-$n" type veth peer name eth0 netns "${NS[$n]}"
  ip link set "v-$n" master br0 up
  inns "$n" sh -c "ip link set lo up; ip addr add 10.9.0.$i/24 dev eth0; ip link set eth0 up"
  i=$((i+1))
done
inns alpha ip route add default via 10.9.0.254
ip link add wan0 netns "${NS[exit]}" type veth peer name eth0 netns "${NS[internet]}"
inns exit sh -c "ip addr add 203.0.113.1/24 dev wan0; ip link set wan0 up; ip route add default via 203.0.113.10"
inns internet sh -c "ip link set lo up; ip addr add 203.0.113.10/24 dev eth0; ip link set eth0 up; ip addr add $TARGET/32 dev lo"

node "$HERE/tests/local-relay.mjs" 7791 10.9.0.254 > "$W/relay.log" 2>&1 & pids+=($!)
wait_for "$W/relay.log" listening 10

echo "=== owner: network + approvals"
"$BIN" --state "$W/owner" owner create --net home --relay "$RELAY" > /dev/null
OWNER=$(python3 -c "import json;print(json.load(open('$W/owner/config.json'))['owner'])")
for n in alpha exit; do
  "$BIN" --state "$W/$n" init --owner "$OWNER" --net home --relay "$RELAY" > /dev/null
  "$BIN" --state "$W/owner" owner approve "$("$BIN" --state "$W/$n" id | awk '/^hex/{print $2}')" --name "$n"
done

# Like a Docker host: iptables FORWARD policy DROP
inns exit iptables -P FORWARD DROP
TOOLNET_SOCKET=$W/exit.sock inns exit "$BIN" --state "$W/exit" up --offline --advertise-exit-node > "$W/exit.log" 2>&1 & pids+=($!)
export TOOLNET_SOCKET=$W/alpha.sock
inns alpha "$BIN" --state "$W/alpha" up --offline --exit-node exit > "$W/alpha.log" 2>&1 & pids+=($!)
wait_for "$W/alpha.log" "connected peer=exit" 30
echo "=== before approval: $TARGET must be unreachable"
if reach $TARGET; then echo "FAIL: reached the internet without an approved exit node"; exit 1; fi
echo "  unreachable, as expected"

echo "=== owner allows exit to be an exit node"
"$BIN" --state "$W/owner" owner routes exit 0.0.0.0/0
wait_for "$W/alpha.log" "using exit node" 20
inns alpha ip rule | grep -E "^52" | sed 's/^/  /'
reach $TARGET && echo "  alpha reaches $TARGET through the exit node" || { echo "FAIL: $TARGET unreachable through the exit"; plain "$W/alpha.log" | tail; exit 1; }
inns internet python3 -c '
import socket
s = socket.socket(); s.bind(("198.51.100.7", 8080)); s.listen(1)
c, a = s.accept(); print(f"  the internet host sees the connection from {a[0]} (the exit node, masqueraded)")
' & SRV=$!
sleep 0.5
inns alpha python3 -c 'import socket; socket.create_connection(("198.51.100.7", 8080), timeout=5).close()'
wait $SRV
reach 10.213.0.2 && echo "  tailnet still fine (10.213.0.2)"
sleep 6
plain "$W/alpha.log" | grep -qE "connection closed" && { echo "FAIL: peer connection dropped after enabling exit routing"; exit 1; }
echo "  peer connection stayed up"

echo "=== exit node switched at runtime over the control socket"
"$BIN" exit-node none | sed 's/^/  /'
if reach $TARGET; then echo "FAIL: still reaching the internet with no exit node"; exit 1; fi
echo "  no exit node: $TARGET unreachable"
if "$BIN" exit-node nosuch > "$W/bad.out" 2>&1; then echo "FAIL: accepted an unknown exit node"; exit 1; fi
grep -o "isn't an exit node" "$W/bad.out" | sed 's/^/  rejected: /'
"$BIN" exit-node exit > /dev/null
reach $TARGET && echo "  exit node back: $TARGET reachable" || { echo "FAIL: exit node not used again"; exit 1; }
python3 -c "import json;p=json.load(open('$W/alpha/prefs.json'));assert p['exit_node']=='exit',p;print('  saved:',p)"

echo "=== owner withdraws the exit node"
"$BIN" --state "$W/owner" owner routes exit
wait_for "$W/alpha.log" "exit node routing off" 20
if reach $TARGET; then echo "FAIL: still reaching the internet"; exit 1; fi
echo "  unreachable again"
echo PASS
INNER
