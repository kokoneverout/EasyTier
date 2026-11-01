"""DNS/STUN fixtures for network-smoke.sh; runs inside its peer namespace."""
import selectors
import socket
import struct
import sys

selector = selectors.DefaultSelector()
for port in (53, 3478):
    sock = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
    sock.bind(('198.51.100.2', port))
    selector.register(sock, selectors.EVENT_READ, port)
tcp = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
tcp.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
tcp.bind(('198.51.100.2', 53))
tcp.listen()
selector.register(tcp, selectors.EVENT_READ, 'tcp')
truncate = '--truncate-dns' in sys.argv


def dns_response(packet, truncated=False):
    end = 12
    while end < len(packet) and packet[end]:
        end += packet[end] + 1
    end += 1
    if end + 4 > len(packet):
        raise ValueError('invalid DNS question')
    qtype = struct.unpack('!H', packet[end:end + 2])[0]
    answer = b''
    if qtype == 1 and not truncated:
        answer = bytes.fromhex('c00c000100010000003c0004') + socket.inet_aton('198.51.100.2')
    return (packet[:2] + struct.pack('!HHHHH', 0x8380 if truncated else 0x8180,
                                  1, bool(answer), 0, 0)
            + packet[12:end + 4] + answer)


def read_exact(connection, size):
    result = b''
    while len(result) < size:
        chunk = connection.recv(size - len(result))
        if not chunk:
            raise EOFError('short DNS TCP frame')
        result += chunk
    return result


while True:
    for key, _ in selector.select():
        sock, port = key.fileobj, key.data
        if port == 'tcp':
            connection, addr = sock.accept()
            try:
                with connection:
                    connection.settimeout(1)
                    length = struct.unpack('!H', read_exact(connection, 2))[0]
                    response = dns_response(read_exact(connection, length))
                    connection.sendall(struct.pack('!H', len(response)) + response)
                print('DNS_TCP_SOURCE', addr[0], flush=True)
            except (EOFError, OSError, ValueError, struct.error):
                pass  # Legacy nodes also probe DNS ports for TCP STUN.
            continue
        packet, addr = sock.recvfrom(65535)
        if port == 3478:
            if len(packet) < 20 or packet[4:8] != bytes.fromhex('2112a442'):
                continue
            mapped = bytes(a ^ b for a, b in zip(socket.inet_aton(addr[0]), packet[4:8]))
            value = struct.pack('!BBH', 0, 1, addr[1] ^ 0x2112) + mapped
            response = (struct.pack('!HH', 0x0101, 12) + packet[4:20]
                        + struct.pack('!HH', 0x0020, 8) + value)
            print('STUN_SOURCE', addr[0], flush=True)
        else:
            response = dns_response(packet, truncate)
            print('DNS_UDP_SOURCE', addr[0], flush=True)
        sock.sendto(response, addr)
