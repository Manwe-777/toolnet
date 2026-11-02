#!/usr/bin/env bash
# Rust <-> JS tool-db compatibility, both directions, on a local ToolDbRelay.
set -euo pipefail
HERE=$(cd "$(dirname "$0")/.." && pwd)
BIN=${BIN:-$HERE/target/debug/toolnet}
W=${WORK:-$(mktemp -d)}
PORT=${PORT:-7792}
RELAY=ws://127.0.0.1:$PORT
mkdir -p "$W"
pids=()
trap 'kill "${pids[@]}" 2>/dev/null || true' EXIT
plain() { sed "s/\x1b\[[0-9;]*m//g" "$1"; }
wait_for() { for _ in $(seq "$3"); do plain "$1" | grep -qE "$2" && return 0; sleep 1; done; echo "TIMEOUT /$2/ in $1"; plain "$1" | tail; return 1; }

node "$HERE/tests/local-relay.mjs" "$PORT" > "$W/relay.log" 2>&1 & pids+=($!)
wait_for "$W/relay.log" listening 10

echo "=== Rust: owner creates, approves alpha, alpha comes online"
"$BIN" --state "$W/owner" owner create --net home --relay "$RELAY" > /dev/null
OWNER=$(python3 -c "import json;print(json.load(open('$W/owner/config.json'))['owner'])")
"$BIN" --state "$W/alpha" init --owner "$OWNER" --net home --relay "$RELAY" > /dev/null
"$BIN" --state "$W/owner" owner approve "$("$BIN" --state "$W/alpha" id | awk '/device/{print $2}')" --name alpha
"$BIN" --state "$W/alpha" up --no-tun --offline > "$W/alpha.log" 2>&1 & pids+=($!)
wait_for "$W/alpha.log" "published node info" 20

echo "=== JS: reads everything, then approves gamma"
"$BIN" --state "$W/gamma" init --owner "$OWNER" --net home --relay "$RELAY" > /dev/null
GAMMA_HEX=$("$BIN" --state "$W/gamma" id | awk '/^hex/{print $2}')
node "$HERE/tests/compat.mjs" "$RELAY" "$W/owner" home "$GAMMA_HEX" gamma 10.213.0.50

echo "=== Rust: gamma sees the JS approval"
"$BIN" --state "$W/gamma" status | tee "$W/gamma-status.txt"
grep -q "(Member)" "$W/gamma-status.txt" && grep -q "gamma .*10.213.0.50" "$W/gamma-status.txt"
echo "PASS"
