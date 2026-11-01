#!/usr/bin/env bash
# Usage: sudo bash network-smoke.sh OLD_BIN_DIR NEW_BIN_DIR OUTPUT_DIR
# Requires Linux, iproute2, util-linux, ping, Python 3 and /dev/net/tun.
set -euo pipefail
old=$(realpath "${1:?old binary directory required}")
new=$(realpath "${2:?candidate binary directory required}")
mkdir -p "${3:?output directory required}"
out=$(realpath "$3")
server=$(realpath "$(dirname "$0")/control_server.py")
for dir in "$old" "$new"; do
  test -x "$dir/easytier-core" && test -x "$dir/easytier-cli"
done
# Every network and resolver mutation is confined to these new namespaces.
exec unshare --mount --net bash -s -- "$old" "$new" "$out" "$server" <<'ISOLATED'
set -euo pipefail
old=$1 new=$2 out=$3 server=$4
mount --make-rprivate /
ip link set lo up
unshare --net sleep 600 &
peer_ns=$!
cleanup() {
  for pid in "${candidate_pid:-}" "${peer_pid:-}" "${control_pid:-}" "$peer_ns"; do
    if [[ -n $pid ]]; then kill "$pid" 2>/dev/null || true; fi
  done
}
trap cleanup EXIT
sleep 0.5
ip link add physical0 type veth peer name physical1
ip link set physical1 netns "$peer_ns"
ip addr add 192.0.2.1/24 dev physical0
ip -6 addr add fd12::1/64 dev physical0
ip link set physical0 up
nsenter -t "$peer_ns" -n ip link set lo up
nsenter -t "$peer_ns" -n ip addr add 192.0.2.2/24 dev physical1
nsenter -t "$peer_ns" -n ip addr add 198.51.100.2/32 dev physical1
nsenter -t "$peer_ns" -n ip -6 addr add fd12::2/64 dev physical1
nsenter -t "$peer_ns" -n ip link set physical1 up
ip route add default via 192.0.2.2 dev physical0
ip -6 route add default via fd12::2 dev physical0
printf 'nameserver 198.51.100.2\noptions timeout:1 attempts:1\n' > "$out/resolv.conf"
mount --bind "$out/resolv.conf" /etc/resolv.conf

for proto in tcp quic; do
  dns_options=()
  if [[ $proto == quic ]]; then dns_options=(--truncate-dns); fi
  nsenter -t "$peer_ns" -n python3 "$server" "${dns_options[@]}" >"$out/$proto-control.log" 2>&1 &
  control_pid=$!
  nsenter -t "$peer_ns" -n "$old/easytier-core" \
    --network-name alignment-isolated --network-secret isolated \
    --ipv4 10.253.2.1/24 --ipv6 fd53::1/64 --dev-name oldtun \
    --listeners "$proto://0.0.0.0:31334" --rpc-portal 31335 \
    --bind-device true --disable-p2p true --disable-udp-hole-punching true \
    >"$out/$proto-old.log" 2>&1 &
  peer_pid=$!
  "$new/easytier-core" \
    --network-name alignment-isolated --network-secret isolated \
    --ipv4 10.253.2.2/24 --ipv6 fd53::2/64 --dev-name newtun \
    --no-listener --peers "$proto://transport.alignment.test:31334" --rpc-portal 31336 \
    --bind-device true --disable-p2p true --disable-udp-hole-punching true \
    --manual-routes 0.0.0.0/1 128.0.0.0/1 --manual-routes-v6 ::/1 8000::/1 \
    --stun-servers 198.51.100.2:3478 --tcp-stun-servers --stun-servers-v6 \
    >"$out/$proto-new.log" 2>&1 &
  candidate_pid=$!
  connected=false
  for attempt in {1..20}; do
    sleep 1
    if ping -q -c 1 -W 1 10.253.2.1 >"$out/$proto-ping4.log" 2>&1; then connected=true; break; fi
  done
  if [[ $connected != true ]]; then tail -30 "$out/$proto-new.log"; exit 1; fi
  ping -6 -q -c 3 -W 2 fd53::1 >"$out/$proto-ping6.log"
  ip route get 198.51.100.2 >"$out/$proto-transport-route.txt"
  grep -q 'dev newtun' "$out/$proto-transport-route.txt"
  "$new/easytier-cli" -p 127.0.0.1:31336 -o json peer >"$out/$proto-peer.json"
  echo "PASS $proto dual-stack payload with defaults captured by VPN"
  ip route del 128.0.0.0/1 dev newtun
  ip -6 route del 8000::/1 dev newtun
  healed=false
  for attempt in {1..20}; do
    sleep 1
    if ip route show dev newtun | grep -q '128.0.0.0/1' && ip -6 route show dev newtun | grep -q '8000::/1'; then healed=true; break; fi
  done
  [[ $healed == true ]]
  echo "PASS $proto removed IPv4 and IPv6 routes restored"
  grep -q 'DNS_UDP_SOURCE 192.0.2.1' "$out/$proto-control.log"
  grep -q 'STUN_SOURCE 192.0.2.1' "$out/$proto-control.log"
  if [[ $proto == quic ]]; then grep -q 'DNS_TCP_SOURCE 192.0.2.1' "$out/$proto-control.log"; fi
  echo "PASS $proto control DNS/STUN use physical source (QUIC also exercises DNS TCP fallback)"
  kill "$candidate_pid" "$peer_pid" "$control_pid"
  wait "$candidate_pid" "$peer_pid" "$control_pid" 2>/dev/null || true
  candidate_pid= peer_pid= control_pid=
  sleep 1
done
ISOLATED
