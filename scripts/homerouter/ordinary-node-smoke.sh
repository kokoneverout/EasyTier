#!/usr/bin/env bash
# Run on Linux ARM64 as root. Only disposable namespaces are changed.
set -euo pipefail
legacy=$(realpath "${1:?legacy binary directory}")
candidate=$(realpath "${2:?candidate binary directory}")
mkdir -p "${3:?output directory}"
out=$(realpath "$3")
exec unshare --mount --net bash -s -- "$legacy" "$candidate" "$out" <<'ISOLATED'
set -euo pipefail
legacy=$1 candidate=$2 out=$3
mount --make-rprivate /
ip link set lo up
unshare --net sleep 600 &
peer_ns=$!
cleanup() {
  result=$?
  if [[ $result != 0 ]]; then
    tail -60 "$out/"*.log 2>/dev/null || true
  fi
  for pid in "${client_pid:-}" "${server_pid:-}" "$peer_ns"; do
    if [[ -n $pid ]]; then kill "$pid" 2>/dev/null || true; fi
  done
  return "$result"
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
nsenter -t "$peer_ns" -n ip -6 addr add fd12::2/64 dev physical1
nsenter -t "$peer_ns" -n ip link set physical1 up
ip route add default via 192.0.2.2 dev physical0
ip -6 route add default via fd12::2 dev physical0
ip route show default > "$out/default4-before"
ip -6 route show default > "$out/default6-before"
sha256sum /etc/resolv.conf > "$out/dns-before"
sleep 2
for direction in candidate-client candidate-server; do
  client=$candidate server=$legacy
  if [[ $direction == candidate-server ]]; then client=$legacy server=$candidate; fi
  for transport in tcp4 tcp6 udp4 udp6; do
    proto=${transport:0:3}
    endpoint=192.0.2.2
    if [[ $transport == *6 ]]; then endpoint='[fd12::2]'; fi
    label=$direction-$transport
    nsenter -t "$peer_ns" -n "$server/easytier-core" \
      --network-name router-ci --network-secret isolated \
      --ipv4 10.253.3.1/24 --ipv6 fd53::1/64 --dev-name servertun \
      --listeners "$proto://$endpoint:31334" --rpc-portal 31335 \
      --bind-device true --disable-p2p true --disable-udp-hole-punching true \
      >"$out/$label-server.log" 2>&1 &
    server_pid=$!
    "$client/easytier-core" \
      --network-name router-ci --network-secret isolated \
      --ipv4 10.253.3.2/24 --ipv6 fd53::2/64 --dev-name clienttun \
      --no-listener --peers "$proto://$endpoint:31334" --rpc-portal 31336 \
      --bind-device true --disable-p2p true --disable-udp-hole-punching true \
      >"$out/$label-client.log" 2>&1 &
    client_pid=$!
    connected=false
    for attempt in {1..20}; do
      sleep 1
      kill -0 "$client_pid" "$server_pid"
      if ping -q -c 1 -W 1 10.253.3.1 >"$out/$label-ping4.log" 2>&1; then connected=true; break; fi
    done
    [[ $connected == true ]]
    ping -6 -q -c 3 -W 2 fd53::1 >"$out/$label-ping6.log"
    nsenter -t "$peer_ns" -n ping -q -c 3 -W 2 10.253.3.2 >"$out/$label-reverse4.log"
    nsenter -t "$peer_ns" -n ping -6 -q -c 3 -W 2 fd53::2 >"$out/$label-reverse6.log"
    "$client/easytier-cli" -p 127.0.0.1:31336 -o json peer >"$out/$label-peer.json"
    ip route show default > "$out/default4-after"
    ip -6 route show default > "$out/default6-after"
    sha256sum /etc/resolv.conf > "$out/dns-after"
    cmp "$out/default4-before" "$out/default4-after"
    cmp "$out/default6-before" "$out/default6-after"
    cmp "$out/dns-before" "$out/dns-after"
    echo "PASS $label: bidirectional IPv4/IPv6 payload, unchanged default routes and DNS"
    kill "$client_pid" "$server_pid"
    wait "$client_pid" "$server_pid" 2>/dev/null || true
    client_pid= server_pid=
    sleep 1
  done
done
ISOLATED
