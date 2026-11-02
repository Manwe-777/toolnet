#!/usr/bin/env bash
# NAT traversal in namespaces: two devices behind NAT routers, meeting through
# a local iroh relay (with QUIC address discovery) and a local ToolDB relay on
# a fake internet. Unprivileged and offline.
#
#   internet (outer ns, 198.51.100.0/24): ToolDB relay, iroh relay  .254
#   homeR 198.51.100.1  NAT  -> home 192.168.10.2
#   lteR  198.51.100.2  NAT  -> lte  192.168.20.2
#
# The routers behave like the carrier NATs measured on LTE <-> home:
# - masquerade (consistent mapping, port preserving), replies only
# - inbound packets that arrive with a low TTL are dropped: a TTL-5 packet
#   opens its own NAT but dies "on the way", like on the real path
# - BLOCK=1 (default): an unsolicited inbound UDP packet blocks that flow
#   both ways for 40 s (what made plain hole punching fail from LTE)
#
#   EXPECT=direct (default): the devices must get a direct path
#   EXPECT=relay: they must stay relayed, e.g. with TOOLNET_PUNCH_HOLD=0 to
#   show that iroh alone can't get through these NATs
set -euo pipefail
HERE=$(cd "$(dirname "$0")/.." && pwd)
BIN=${BIN:-$HERE/target/debug/toolnet}
RELAY_BIN=${IROH_RELAY:-$(command -v iroh-relay || true)}
[ -x "$RELAY_BIN" ] || { echo "needs iroh-relay (cargo install iroh-relay --version 1.4.1 --features server), or IROH_RELAY=path"; exit 1; }
W=${WORK:-$(mktemp -d)}
mkdir -p "$W"
export BLOCK=${BLOCK:-1} EXPECT=${EXPECT:-direct} WAIT=${WAIT:-60}

# A throwaway CA and a certificate for the relay's IP (QUIC address discovery
# needs TLS even when the relay itself is plain HTTP)
if [ ! -e "$W/ca.pem" ]; then
  openssl req -x509 -newkey ec -pkeyopt ec_paramgen_curve:P-256 -nodes -days 2 -subj /CN=toolnet-test-ca \
    -addext basicConstraints=critical,CA:TRUE -keyout "$W/ca.key" -out "$W/ca.pem" 2>/dev/null
  openssl req -newkey ec -pkeyopt ec_paramgen_curve:P-256 -nodes -subj /CN=198.51.100.254 \
    -keyout "$W/relay.key" -out "$W/relay.csr" 2>/dev/null
  printf 'subjectAltName=IP:198.51.100.254\nbasicConstraints=CA:FALSE\n' > "$W/ext.cnf"
  openssl x509 -req -in "$W/relay.csr" -CA "$W/ca.pem" -CAkey "$W/ca.key" -CAcreateserial -days 2 \
    -extfile "$W/ext.cnf" -out "$W/relay.pem" 2>/dev/null
fi
cat > "$W/relay.toml" <<TOML
http_bind_addr = "198.51.100.254:3340"
enable_quic_addr_discovery = true
[tls]
cert_mode = "Manual"
manual_cert_path = "$W/relay.pem"
manual_key_path = "$W/relay.key"
hostname = "198.51.100.254"
quic_bind_addr = "198.51.100.254:7842"
TOML

exec unshare -Urn --kill-child bash -s "$BIN" "$W" "$HERE" "$RELAY_BIN" <<'INNER'
set -euo pipefail
BIN=$1 W=$2 HERE=$3 RELAY_BIN=$4
NOSTR=ws://198.51.100.254:7791
IROH=http://198.51.100.254:3340
pids=()
trap 'kill "${pids[@]}" 2>/dev/null || true' EXIT
plain() { sed "s/\x1b\[[0-9;]*m//g" "$1"; }
wait_for() { for _ in $(seq "$3"); do plain "$1" | grep -qE "$2" && return 0; sleep 1; done; echo "TIMEOUT /$2/ in $1"; plain "$1" | grep -vE "low-TTL|punching" | tail -15; exit 1; }
inns() { local ns=$1; shift; nsenter -t "${NS[$ns]}" -n "$@"; }
declare -A NS

ip link set lo up
ip link add inet0 type bridge && ip addr add 198.51.100.254/24 dev inet0 && ip link set inet0 up
for n in homeR lteR home lte; do unshare -n sleep infinity & NS[$n]=$!; pids+=($!); done
sleep 0.3
i=1
for site in home lte; do
  r=${site}R lan=192.168.$((i * 10))
  ip link add "v-$r" type veth peer name eth0 netns "${NS[$r]}"
  ip link set "v-$r" master inet0 up
  ip link add lan0 netns "${NS[$r]}" type veth peer name eth0 netns "${NS[$site]}"
  inns "$r" sh -c "ip link set lo up; ip addr add 198.51.100.$i/24 dev eth0; ip link set eth0 up
    ip addr add $lan.1/24 dev lan0; ip link set lan0 up; ip route add default via 198.51.100.254
    sysctl -qw net.ipv4.ip_forward=1"
  inns "$site" sh -c "ip link set lo up; ip addr add $lan.2/24 dev eth0; ip link set eth0 up; ip route add default via $lan.1"
  block=""; ttl='iifname "eth0" ip saddr != 198.51.100.254 ip ttl < 50 drop'
  [ "${TTLDROP:-1}" = 1 ] || ttl="iifname \"eth0\" counter"
  if [ "$BLOCK" = 1 ]; then block='
      iifname "eth0" ip saddr . udp sport . udp dport @blocked update @blocked { ip saddr . udp sport . udp dport } drop
      iifname "eth0" meta l4proto udp ct state new ip saddr != 198.51.100.254 add @blocked { ip saddr . udp sport . udp dport } drop'
  fi
  inns "$r" nft -f - <<NFT
table ip nat {
  chain post { type nat hook postrouting priority 100; oifname "eth0" masquerade; }
}
table ip carrier {
  set blocked { type ipv4_addr . inet_service . inet_service; flags dynamic, timeout; timeout 40s; }
  chain pre {
    type filter hook prerouting priority -150;
    $ttl$block
  }
  chain post {
    type filter hook postrouting priority 200;
    oifname "eth0" ip daddr . udp dport . udp sport @blocked drop
  }
}
NFT
  i=$((i + 1))
done

node "$HERE/tests/local-relay.mjs" 7791 198.51.100.254 > "$W/nostr.log" 2>&1 & pids+=($!)
"$RELAY_BIN" --dev -c "$W/relay.toml" > "$W/iroh-relay.log" 2>&1 & pids+=($!)
wait_for "$W/nostr.log" listening 10
sleep 1

if [ -n "${RAWTEST:-}" ]; then # debugging the harness: raw UDP, no toolnet
  T=$(( $(date +%s) + 2 ))
  inns home python3 "$RAWTEST" 40000 198.51.100.2:40000 $T "${RAWSCHED:-0:5,3:64}" > "$W/raw-home.log" 2>&1 &
  inns lte python3 "$RAWTEST" 40000 198.51.100.1:40000 $T "${RAWSCHED:-0:5,3:64}" > "$W/raw-lte.log" 2>&1
  sleep 1; sed 's/^/  home: /' "$W/raw-home.log"; sed 's/^/  lte: /' "$W/raw-lte.log"; exit 0
fi

echo "=== owner: network + approvals (BLOCK=$BLOCK, EXPECT=$EXPECT)"
"$BIN" --state "$W/owner" owner create --net home --relay "$NOSTR" > /dev/null
OWNER=$(python3 -c "import json;print(json.load(open('$W/owner/config.json'))['owner'])")
for n in home lte; do
  "$BIN" --state "$W/$n" init --owner "$OWNER" --net home --relay "$NOSTR" > /dev/null
  "$BIN" --state "$W/owner" owner approve "$("$BIN" --state "$W/$n" id | awk '/^hex/{print $2}')" --name "$n" > /dev/null
done

for n in home lte; do
  inns "$n" env RUST_LOG=${NAT_LOG:-info,toolnet=debug} TOOLNET_PUNCH_HOLD=${TOOLNET_PUNCH_HOLD:-1} "$BIN" --state "$W/$n" up --no-tun \
    --iroh-relay "$IROH" --relay-ca "$W/ca.pem" > "$W/$n.log" 2>&1 & pids+=($!)
done
wait_for "$W/lte.log" "connected peer=home" 30
wait_for "$W/lte.log" 'published node info addrs=\[.*198\.51\.100\.2:' 20
echo "  connected through the relay; lte knows its public address"
t0=$(date +%s)
if [ "$EXPECT" = direct ]; then
  wait_for "$W/lte.log" "path DIRECT Ip\(198\.51\.100\.1:" "$WAIT"
  echo "  direct path after $(( $(date +%s) - t0 )) s"
  wait_for "$W/home.log" "path DIRECT Ip\(198\.51\.100\.2:" 15
else
  sleep "$WAIT"
  if plain "$W/lte.log" | grep -qE "path DIRECT Ip\(198\.51\.100\.1:"; then echo "FAIL: went direct"; exit 1; fi
  echo "  still relayed after $WAIT s, as expected"
fi
echo PASS
INNER
