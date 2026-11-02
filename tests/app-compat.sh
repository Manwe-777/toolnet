#!/usr/bin/env bash
# The owner app's data logic (app/src/lib/net.js) against the Rust daemon, on
# a local ToolDbRelay, no internet:
#   Rust creates the network -> JS approves two devices (one from a QR payload)
#   -> the Rust devices connect -> JS sets routes, Rust lists them
#   -> JS removes one: the other drops it, the removed one is not a member
# Needs app/node_modules (cd app && npm install) and a built ../tool-db.
set -euo pipefail
HERE=$(cd "$(dirname "$0")/.." && pwd)
BIN=${BIN:-$HERE/target/debug/toolnet}
W=${WORK:-$(mktemp -d)}
PORT=${PORT:-7793}
RELAY=ws://127.0.0.1:$PORT
mkdir -p "$W"
pids=()
trap 'kill "${pids[@]}" 2>/dev/null || true' EXIT
log() { echo "=== $*"; }
plain() { sed "s/\x1b\[[0-9;]*m//g" "$1"; }
wait_for() { # file pattern seconds
  for _ in $(seq "$3"); do plain "$1" | grep -qE "$2" && return 0; sleep 1; done
  echo "TIMEOUT waiting for /$2/ in $1"; plain "$1" | tail -20; return 1
}
js() { node "$HERE/tests/app-compat.mjs" "$RELAY" "$W/owner" home "$@"; }

[ -d "$HERE/app/node_modules/tool-db" ] || { echo "run: (cd app && npm install)"; exit 1; }
node "$HERE/tests/app-compat.mjs" selftest

node "$HERE/tests/local-relay.mjs" "$PORT" > "$W/relay.log" 2>&1 & pids+=($!)
wait_for "$W/relay.log" listening 10

log "Rust: owner creates the network"
"$BIN" --state "$W/owner" owner create --net home --relay "$RELAY" > /dev/null
OWNER=$(python3 -c "import json;print(json.load(open('$W/owner/config.json'))['owner'])")
for d in alpha beta; do
  "$BIN" --state "$W/$d" init --owner "$OWNER" --net home --relay "$RELAY" > /dev/null
done
A_NPUB=$("$BIN" --state "$W/alpha" id | awk '/^device/{print $2}')
B_HEX=$("$BIN" --state "$W/beta" id | awk '/^hex/{print $2}')

log "JS: approves alpha from its QR payload, beta from its hex key"
js approve "toolnet:join?device=$A_NPUB&name=alpha"
js approve "$B_HEX" beta
if js approve "$B_HEX" alpha 2>/dev/null; then echo "FAIL: a duplicate name was accepted"; exit 1; fi

"$BIN" --state "$W/owner" owner list | tee "$W/list1.txt"
grep -qE "alpha +10\.213\.0\.1 " "$W/list1.txt" && grep -qE "beta +10\.213\.0\.2 " "$W/list1.txt"

log "Rust: alpha and beta come up and connect"
RUST_LOG=toolnet=info,iroh=warn,nostr_relay_pool=error "$BIN" --state "$W/alpha" up --no-tun --offline > "$W/alpha.log" 2>&1 & pids+=($!)
"$BIN" --state "$W/beta" up --no-tun --offline > "$W/beta.log" 2>&1 & pids+=($!)
wait_for "$W/alpha.log" "member of network" 20
wait_for "$W/alpha.log" "connected peer=beta" 40
wait_for "$W/beta.log" "connected" 40
wait_for "$W/alpha.log" "ping DIRECT" 20
log "alpha <-> beta connected"
wait_for "$W/beta.log" "published node info" 20
js show 2

log "JS: routes for alpha (normalized), Rust lists them"
js routes alpha 192.168.50.7/24 0.0.0.0/0
"$BIN" --state "$W/owner" owner list | tee "$W/list2.txt"
grep -qF 'allowed=["192.168.50.0/24", "0.0.0.0/0"]' "$W/list2.txt"
js routes alpha
"$BIN" --state "$W/owner" owner list | tee "$W/list3.txt"
if grep -q "allowed=" "$W/list3.txt"; then echo "FAIL: routes not cleared"; exit 1; fi
js routes alpha 192.168.50.0/24

log "JS: removes beta"
js remove beta
wait_for "$W/alpha.log" "peer removed" 30
wait_for "$W/beta.log" "not a member" 30
js expect-epoch 2
js not-member "$W/beta"
"$BIN" --state "$W/owner" owner list | tee "$W/list4.txt"
grep -q "epoch 2, 1 members" "$W/list4.txt" && ! grep -q "beta" "$W/list4.txt"
grep -qF 'allowed=["192.168.50.0/24"]' "$W/list4.txt"
"$BIN" --state "$W/alpha" status | tee "$W/alpha-status.txt"
grep -q "(Member)" "$W/alpha-status.txt"
"$BIN" --state "$W/beta" status | tee "$W/beta-status.txt"
grep -q "NotMember" "$W/beta-status.txt"
echo "PASS (work dir $W)"
