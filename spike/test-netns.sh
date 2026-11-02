#!/usr/bin/env bash
# Two toolnet peers in separate network namespaces joined by a veth pair.
# Unprivileged (user namespaces), offline (direct path only, no relay).
#
#   ./test-netns.sh            # uses target/debug/toolnet-spike
#   BIN=... WORK=... ./test-netns.sh
set -euo pipefail
HERE=$(cd "$(dirname "$0")" && pwd)
BIN=${BIN:-$HERE/target/debug/toolnet-spike}
WORK=${WORK:-$(mktemp -d)}
mkdir -p "$WORK"

exec unshare -Urn --kill-child bash -s "$BIN" "$WORK" <<'INNER'
set -euo pipefail
BIN=$1 WORK=$2
cleanup() { kill $(jobs -p) 2>/dev/null || true; }
trap cleanup EXIT

# Two child network namespaces, held open by sleepers
unshare -n sleep infinity & A=$!
unshare -n sleep infinity & B=$!
sleep 0.3
ip link add va type veth peer name vb
ip link set va netns $A
ip link set vb netns $B
nsenter -t $A -n sh -c 'ip link set lo up; ip addr add 10.9.0.1/24 dev va; ip link set va up'
nsenter -t $B -n sh -c 'ip link set lo up; ip addr add 10.9.0.2/24 dev vb; ip link set vb up'

IDA=$("$BIN" keygen "$WORK/a.key")
IDB=$("$BIN" keygen "$WORK/b.key")
echo "A=$IDA"
echo "B=$IDB"

nsenter -t $A -n "$BIN" up --key "$WORK/a.key" --ip 10.213.0.1 --offline \
  --bind 10.9.0.1:7777 --peer "$IDB=10.213.0.2" --peer-addr "$IDB=10.9.0.2:7777" \
  >"$WORK/a.log" 2>&1 &
nsenter -t $B -n "$BIN" up --key "$WORK/b.key" --ip 10.213.0.2 --offline \
  --bind 10.9.0.2:7777 --peer "$IDA=10.213.0.1" --peer-addr "$IDA=10.9.0.1:7777" \
  >"$WORK/b.log" 2>&1 &
sleep 3

echo "--- ping A -> B over the tunnel"
nsenter -t $A -n ping -c 5 -i 0.2 10.213.0.2
echo "--- ping with 1100 byte payload"
nsenter -t $A -n ping -c 3 -i 0.2 -s 1100 10.213.0.2

echo "--- TCP transfer B -> A, 50 MB"
nsenter -t $A -n python3 -c '
import socket, time
s = socket.socket(); s.bind(("10.213.0.1", 5001)); s.listen(1)
c, _ = s.accept(); t = time.time(); n = 0
while (d := c.recv(1 << 16)): n += len(d)
dt = time.time() - t
print(f"received {n/1e6:.1f} MB in {dt:.2f}s = {n*8/dt/1e6:.0f} Mbit/s")
' & SRV=$!
sleep 0.5
nsenter -t $B -n python3 -c '
import socket
s = socket.create_connection(("10.213.0.1", 5001)); buf = b"x" * (1 << 16)
for _ in range(800): s.sendall(buf)
s.close()
'
wait $SRV

sleep 5
echo "--- A log"; cat "$WORK/a.log"
echo "--- B log"; cat "$WORK/b.log"
INNER
