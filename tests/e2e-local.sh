#!/usr/bin/env bash
# End to end on one machine, no privileges, no internet: a local ToolDB relay,
# an owner and two devices running `toolnet up --no-tun --offline`.
#   create network -> approve both -> they connect and ping
#   -> remove one -> group key rotates, the other drops it
set -euo pipefail
HERE=$(cd "$(dirname "$0")/.." && pwd)
BIN=${BIN:-$HERE/target/debug/toolnet}
W=${WORK:-$(mktemp -d)}
PORT=${PORT:-7791}
RELAY=ws://127.0.0.1:$PORT
mkdir -p "$W"
pids=()
cleanup() { kill "${pids[@]}" 2>/dev/null || true; }
trap cleanup EXIT
log() { echo "=== $*"; }
plain() { sed "s/\x1b\[[0-9;]*m//g" "$1"; }
wait_for() { # file pattern seconds
  for _ in $(seq "$3"); do plain "$1" | grep -qE "$2" && return 0; sleep 1; done
  echo "TIMEOUT waiting for /$2/ in $1"; plain "$1" | tail -20; return 1
}

node "$HERE/tests/local-relay.mjs" "$PORT" > "$W/relay.log" 2>&1 & pids+=($!)
wait_for "$W/relay.log" "listening" 10

log "owner creates the network"
"$BIN" --state "$W/owner" owner create --net home --relay "$RELAY"
OWNER=$(plain <("$BIN" --state "$W/owner" owner list 2>/dev/null) >/dev/null; python3 -c "import json;print(json.load(open('$W/owner/config.json'))['owner'])")

for d in alpha beta; do
  "$BIN" --state "$W/$d" init --owner "$OWNER" --net home --relay "$RELAY" > /dev/null
done
A=$("$BIN" --state "$W/alpha" id | awk '/device/ {print $2}')
B=$("$BIN" --state "$W/beta" id | awk '/device/ {print $2}')

log "alpha starts before approval (should wait)"
RUST_LOG=${ALPHA_LOG:-toolnet=info,iroh=warn,nostr_relay_pool=error} "$BIN" --state "$W/alpha" up --no-tun --offline > "$W/alpha.log" 2>&1 & pids+=($!)
wait_for "$W/alpha.log" "not a member" 20

log "owner approves both"
"$BIN" --state "$W/owner" owner approve "$A" --name alpha
"$BIN" --state "$W/owner" owner approve "$B" --name beta
"$BIN" --state "$W/beta" up --no-tun --offline > "$W/beta.log" 2>&1 & pids+=($!)

wait_for "$W/alpha.log" "member of network" 20
wait_for "$W/alpha.log" "connected peer=beta" 40
wait_for "$W/beta.log" "connected" 40
wait_for "$W/alpha.log" "ping DIRECT" 20
log "alpha <-> beta connected"
plain "$W/alpha.log" | grep -E "ping|path" | head -3

log "status as beta"
"$BIN" --state "$W/beta" status

log "owner removes beta"
"$BIN" --state "$W/owner" owner remove beta
wait_for "$W/alpha.log" "peer removed" 30
wait_for "$W/beta.log" "not a member" 30
log "alpha dropped beta, beta lost membership"

log "owner list"
"$BIN" --state "$W/owner" owner list
echo "PASS (work dir $W)"
