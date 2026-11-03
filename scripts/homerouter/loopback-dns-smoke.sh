#!/usr/bin/env bash
# Exercise OpenWrt-style loopback DNS without changing the runner network.
set -euo pipefail
bin=$(realpath "${1:?binary directory}")
mkdir -p "${2:?output directory}"
out=$(realpath "$2")
fixture=$(realpath "$(dirname "$0")/../upstream-alignment/control_server.py")
exec unshare --mount --net bash -s -- "$bin" "$out" "$fixture" <<'ISOLATED'
set -euo pipefail
bin=$1 out=$2 fixture=$3
mount --make-rprivate /
ip link set lo up
printf 'nameserver 127.0.0.53\noptions timeout:1 attempts:1\n' > "$out/resolv.conf"
mount --bind "$out/resolv.conf" /etc/resolv.conf
sed 's/198\.51\.100\.2/127.0.0.53/g' "$fixture" > "$out/loopback-server.py"
cleanup() {
  result=$?
  if [[ $result != 0 ]]; then tail -70 "$out/"*.log 2>/dev/null || true; fi
  for pid in "${client_pid:-}" "${server_pid:-}" "${dns_pid:-}"; do
    if [[ -n $pid ]]; then kill "$pid" 2>/dev/null || true; fi
  done
  return "$result"
}
trap cleanup EXIT
for mode in udp tcp-fallback; do
  options=()
  if [[ $mode == tcp-fallback ]]; then options=(--truncate-dns); fi
  python3 "$out/loopback-server.py" "${options[@]}" > "$out/$mode-dns.log" 2>&1 &
  dns_pid=$!
  "$bin/easytier-core" --hostname LoopbackDNSServer \
    --network-name router-dns-ci --network-secret isolated --ipv4 10.253.4.1/24 \
    --no-tun true --listeners tcp://127.0.0.53:31334 --rpc-portal 31335 \
    --bind-device true --disable-p2p true --disable-udp-hole-punching true \
    --stun-servers 127.0.0.53:3478 --stun-servers-v6 --tcp-stun-servers \
    > "$out/$mode-server.log" 2>&1 &
  server_pid=$!
  "$bin/easytier-core" --hostname LoopbackDNSClient \
    --network-name router-dns-ci --network-secret isolated --ipv4 10.253.4.2/24 \
    --no-tun true --no-listener --peers tcp://loopback.router.test:31334 --rpc-portal 31336 \
    --bind-device true --disable-p2p true --disable-udp-hole-punching true \
    --stun-servers 127.0.0.53:3478 --stun-servers-v6 --tcp-stun-servers \
    > "$out/$mode-client.log" 2>&1 &
  client_pid=$!
  connected=false
  for attempt in {1..20}; do
    sleep 1
    kill -0 "$client_pid" "$server_pid" "$dns_pid"
    "$bin/easytier-cli" -p 127.0.0.1:31336 -o json peer > "$out/$mode-peer.json"
    if grep -q LoopbackDNSServer "$out/$mode-peer.json"; then connected=true; break; fi
  done
  [[ $connected == true ]]
  grep -q 'DNS_UDP_SOURCE 127\.' "$out/$mode-dns.log"
  if [[ $mode == tcp-fallback ]]; then grep -q 'DNS_TCP_SOURCE 127\.' "$out/$mode-dns.log"; fi
  echo "PASS loopback DNS $mode: bound-underlay domain peer connected"
  kill "$client_pid" "$server_pid" "$dns_pid"
  wait "$client_pid" "$server_pid" "$dns_pid" 2>/dev/null || true
  client_pid= server_pid= dns_pid=
  sleep 1
done
ISOLATED
