#!/usr/bin/env bash
# The full daemon with TUN devices, in network namespaces. Unprivileged
# (user namespaces) and offline (direct paths, local ToolDB relay).
#
#   outer ns: bridge 10.9.0.254, ToolDB relay, owner commands
#   alpha:    10.9.0.1  (client)
#   router:   10.9.0.2  (advertises 192.168.77.0/24)
#   lan:      192.168.77.10, reachable only from router
#
# Checks: tailnet ping + TCP; alpha reaches the LAN host through the router
# once the owner allows the route, unless alpha is itself on a network with
# that prefix; and not after it's withdrawn.
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
for n in alpha router lan; do unshare -n sleep infinity & NS[$n]=$!; pids+=($!); done
sleep 0.3
i=1
for n in alpha router; do
  ip link add "v-$n" type veth peer name eth0 netns "${NS[$n]}"
  ip link set "v-$n" master br0 up
  inns "$n" sh -c "ip link set lo up; ip addr add 10.9.0.$i/24 dev eth0; ip link set eth0 up"
  i=$((i+1))
done
ip link add lan0 netns "${NS[router]}" type veth peer name eth0 netns "${NS[lan]}"
inns router sh -c "ip addr add 192.168.77.1/24 dev lan0; ip link set lan0 up"
inns lan sh -c "ip link set lo up; ip addr add 192.168.77.10/24 dev eth0; ip link set eth0 up"

node "$HERE/tests/local-relay.mjs" 7791 10.9.0.254 > "$W/relay.log" 2>&1 & pids+=($!)
wait_for "$W/relay.log" listening 10

echo "=== owner: network + approvals"
"$BIN" --state "$W/owner" owner create --net home --relay "$RELAY" > /dev/null
OWNER=$(python3 -c "import json;print(json.load(open('$W/owner/config.json'))['owner'])")
for n in alpha router; do
  "$BIN" --state "$W/$n" init --owner "$OWNER" --net home --relay "$RELAY" > /dev/null
  "$BIN" --state "$W/owner" owner approve "$("$BIN" --state "$W/$n" id | awk '/^hex/{print $2}')" --name "$n"
done

export TOOLNET_SOCKET=$W/alpha.sock
inns alpha "$BIN" --state "$W/alpha" up --offline > "$W/alpha.log" 2>&1 & pids+=($!)
TOOLNET_SOCKET=$W/router.sock inns router "$BIN" --state "$W/router" up --offline --advertise-routes 192.168.77.0/24 > "$W/router.log" 2>&1 & pids+=($!)
wait_for "$W/alpha.log" "connected peer=router" 30
wait_for "$W/router.log" "routing for the tailnet" 10

echo "=== tailnet ping alpha -> router (10.213.0.2)"
inns alpha ping -c 3 -i 0.2 -W 2 10.213.0.2 | tail -2

echo "=== MagicDNS: alpha resolves router.home.toolnet on its own tailnet IP"
inns alpha python3 -c '
import socket, struct
q = b"\x12\x34\x01\x00\x00\x01\x00\x00\x00\x00\x00\x00" + b"".join(bytes([len(l)]) + l.encode() for l in "router.home.toolnet".split(".")) + b"\x00\x00\x01\x00\x01"
s = socket.socket(socket.AF_INET, socket.SOCK_DGRAM); s.settimeout(2)
s.sendto(q, ("10.213.0.1", 53)); r, _ = s.recvfrom(512)
ip = ".".join(map(str, r[-4:])); assert r[3] & 15 == 0 and ip == "10.213.0.2", (r[3] & 15, ip)
print("  router.home.toolnet ->", ip)
'

echo "=== TCP alpha -> router, 50 MB"
inns router python3 -c '
import socket, time
s = socket.socket(); s.bind(("10.213.0.2", 5001)); s.listen(1)
c, _ = s.accept(); t = time.time(); n = 0
while (d := c.recv(1 << 16)): n += len(d)
dt = time.time() - t
print(f"  {n/1e6:.1f} MB in {dt:.2f}s = {n*8/dt/1e6:.0f} Mbit/s")
' & SRV=$!
sleep 0.5
inns alpha python3 -c '
import socket
s = socket.create_connection(("10.213.0.2", 5001)); b = b"x" * 65536
for _ in range(800): s.sendall(b)
s.close()'
wait $SRV

echo "=== LAN host before the owner allows the route (must fail)"
if inns alpha ping -c 1 -W 1 192.168.77.10 > /dev/null 2>&1; then echo "FAIL: reached LAN without approval"; exit 1; fi
echo "  unreachable, as expected"

echo "=== owner allows 192.168.77.0/24 for router"
"$BIN" --state "$W/owner" owner routes router 192.168.77.0/24
wait_for "$W/alpha.log" "route added" 20
inns alpha ip route show table 5213 | sed 's/^/  /'
inns alpha ping -c 3 -i 0.2 -W 2 192.168.77.10 | tail -2

echo "=== alpha joins a local network with the same prefix: it stays local"
inns alpha sh -c "ip link add home0 type dummy; ip addr add 192.168.77.99/24 dev home0; ip link set home0 up"
via=$(inns alpha ip -o route get 192.168.77.10 | grep -o 'dev [^ ]*')
[ "$via" = "dev home0" ] || { echo "FAIL: 192.168.77.10 goes $via"; exit 1; }
echo "  192.168.77.10 -> $via"
inns alpha ip link del home0
via=$(inns alpha ip -o route get 192.168.77.10 | grep -o 'dev [^ ]*')
[ "$via" = "dev toolnet0" ] || { echo "FAIL: 192.168.77.10 goes $via"; exit 1; }
echo "  left it: 192.168.77.10 -> $via"

echo "=== control socket: status, disconnect, connect"
"$BIN" status | sed 's/^/  /'
"$BIN" status --json | python3 -c '
import json, sys
s = json.load(sys.stdin); r = {p["name"]: p for p in s["peers"]}["router"]
assert s["state"] == "member" and s["connected"] and s["ip"] == "10.213.0.1", s
assert r["path"] == "direct" and r["online"] and r["routes"] == ["192.168.77.0/24"], r
print("  json ok: router", r["path"], r["rtt_ms"], "ms")'
"$BIN" disconnect > /dev/null
for _ in $(seq 20); do inns alpha ip link show toolnet0 > /dev/null 2>&1 || break; sleep 0.2; done
inns alpha ip link show toolnet0 > /dev/null 2>&1 && { echo "FAIL: toolnet0 still there"; exit 1; }
[ -z "$(inns alpha ip route show table 5213)" ] || { echo "FAIL: routes left behind"; exit 1; }
"$BIN" status --json | python3 -c 'import json,sys; s=json.load(sys.stdin); assert not s["connected"] and all(p["path"] is None for p in s["peers"]), s'
echo "  disconnected: no toolnet0, no routes, no peers"
"$BIN" connect > /dev/null
for _ in $(seq 30); do inns alpha ping -c 1 -W 1 192.168.77.10 > /dev/null 2>&1 && break; sleep 0.5; done
inns alpha ping -c 2 -i 0.2 -W 2 192.168.77.10 | tail -1 | sed 's/^/  reconnected: /'
inns alpha python3 -c '
import socket
q = b"\x12\x34\x01\x00\x00\x01\x00\x00\x00\x00\x00\x00" + b"".join(bytes([len(l)]) + l.encode() for l in "router.home.toolnet".split(".")) + b"\x00\x00\x01\x00\x01"
s = socket.socket(socket.AF_INET, socket.SOCK_DGRAM); s.settimeout(2)
s.sendto(q, ("10.213.0.1", 53)); r, _ = s.recvfrom(512); assert r[-4:] == bytes([10,213,0,2])
print("  MagicDNS back")'

echo "=== owner withdraws the route"
"$BIN" --state "$W/owner" owner routes router
wait_for "$W/alpha.log" "route removed" 20
if inns alpha ping -c 1 -W 1 192.168.77.10 > /dev/null 2>&1; then echo "FAIL: LAN still reachable"; exit 1; fi
echo "  unreachable again"
echo PASS
INNER
