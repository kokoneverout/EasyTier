import { v4 as uuidv4 } from 'uuid'

export enum NetworkingMethod {
  PublicServer = 0,
  Manual = 1,
  Standalone = 2,
}

export interface SecureModeConfig {
  enabled: boolean
  // Keep protocol compatibility with backend/import-export flows even though the GUI
  // does not render secure-mode or credential inputs.
  local_private_key?: string
  local_public_key?: string
}

export enum AclProtocol {
  Unspecified = 0,
  TCP = 1,
  UDP = 2,
  ICMP = 3,
  ICMPv6 = 4,
  Any = 5,
}

export enum AclAction {
  Noop = 0,
  Allow = 1,
  Drop = 2,
}

export enum AclChainType {
  UnspecifiedChain = 0,
  Inbound = 1,
  Outbound = 2,
  Forward = 3,
}

export interface AclRule {
  name: string
  description: string
  priority: number
  enabled: boolean
  protocol: AclProtocol
  ports: string[]
  source_ips: string[]
  destination_ips: string[]
  source_ports: string[]
  action: AclAction
  rate_limit: number
  burst_limit: number
  stateful: boolean
  source_groups: string[]
  destination_groups: string[]
}

export interface AclChain {
  name: string
  chain_type: AclChainType
  description: string
  enabled: boolean
  rules: AclRule[]
  default_action: AclAction
}

export interface GroupIdentity {
  group_name: string
  group_secret: string
}

export interface GroupInfo {
  declares: GroupIdentity[]
  members: string[]
}

export interface AclV1 {
  chains: AclChain[]
  group?: GroupInfo
}

export interface Acl {
  acl_v1?: AclV1
}

export interface NetworkConfig {
  instance_id: string

  dhcp: boolean
  virtual_ipv4: string
  network_length: number
  // Full static IPv6 CIDR, e.g. fd79:68a2:532c::8/64. Empty/undefined keeps
  // the previous IPv4-only behavior.
  virtual_ipv6?: string
  // DNS server the mobile VPN layer installs (IPv4 or IPv6). Takes priority
  // over Magic DNS on mobile.
  vpn_dns?: string
  hostname?: string
  network_name: string
  network_secret?: string
  credential_file?: string
  secure_mode?: SecureModeConfig

  networking_method: NetworkingMethod

  public_server_url: string
  peer_urls: string[]

  proxy_cidrs: string[]

  enable_vpn_portal: boolean
  vpn_portal_listen_port: number
  vpn_portal_client_network_addr: string
  vpn_portal_client_network_len: number

  advanced_settings: boolean

  listener_urls: string[]
  latency_first: boolean

  dev_name: string

  use_smoltcp?: boolean
  disable_ipv6?: boolean
  ipv6_public_addr_auto?: boolean
  enable_kcp_proxy?: boolean
  disable_kcp_input?: boolean
  enable_quic_proxy?: boolean
  disable_quic_input?: boolean
  disable_p2p?: boolean
  p2p_only?: boolean
  lazy_p2p?: boolean
  bind_device?: boolean
  no_tun?: boolean
  enable_exit_node?: boolean
  relay_all_peer_rpc?: boolean
  need_p2p?: boolean
  multi_thread?: boolean
  proxy_forward_by_system?: boolean
  disable_encryption?: boolean
  disable_tcp_hole_punching?: boolean
  disable_udp_hole_punching?: boolean
  disable_upnp?: boolean
  enable_udp_broadcast_relay?: boolean
  disable_sym_hole_punching?: boolean

  enable_relay_network_whitelist?: boolean
  relay_network_whitelist: string[]

  enable_manual_routes: boolean
  routes: string[]
  routes_v6: string[]

  exit_nodes: string[]

  enable_socks5?: boolean
  socks5_port: number

  mtu: number | null
  instance_recv_bps_limit: number | null
  mapped_listeners: string[]

  enable_magic_dns?: boolean
  enable_private_mode?: boolean

  port_forwards: PortForwardConfig[]
  acl?: Acl
}

export function DEFAULT_NETWORK_CONFIG(): NetworkConfig {
  return {
    instance_id: uuidv4(),

    dhcp: true,
    virtual_ipv4: '',
    network_length: 24,
    network_name: 'easytier',
    network_secret: '',
    credential_file: '',

    networking_method: NetworkingMethod.Manual,
    public_server_url: '',
    peer_urls: [],

    proxy_cidrs: [],

    enable_vpn_portal: false,
    vpn_portal_listen_port: 22022,
    vpn_portal_client_network_addr: '',
    vpn_portal_client_network_len: 24,

    advanced_settings: false,

    listener_urls: [
      'tcp://0.0.0.0:11010',
      'udp://0.0.0.0:11010',
      'wg://0.0.0.0:11011',
    ],
    latency_first: false,
    dev_name: '',

    use_smoltcp: false,
    disable_ipv6: false,
    ipv6_public_addr_auto: false,
    enable_kcp_proxy: false,
    disable_kcp_input: false,
    enable_quic_proxy: false,
    disable_quic_input: false,
    disable_p2p: false,
    p2p_only: false,
    lazy_p2p: false,
    bind_device: true,
    no_tun: false,
    enable_exit_node: false,
    relay_all_peer_rpc: false,
    need_p2p: false,
    multi_thread: true,
    proxy_forward_by_system: false,
    disable_encryption: false,
    disable_tcp_hole_punching: false,
    disable_udp_hole_punching: false,
    disable_upnp: false,
    enable_udp_broadcast_relay: false,
    disable_sym_hole_punching: false,
    enable_relay_network_whitelist: false,
    relay_network_whitelist: [],
    enable_manual_routes: false,
    routes: [],
    routes_v6: [],
    exit_nodes: [],
    enable_socks5: false,
    socks5_port: 1080,
    mtu: null,
    instance_recv_bps_limit: null,
    mapped_listeners: [],
    enable_magic_dns: false,
    enable_private_mode: false,
    port_forwards: [],
    acl: {
      acl_v1: {
        group: {
          declares: [],
          members: [],
        },
        chains: [],
      },
    },
  }
}

function cleanPeerUrls(urls: string[] | undefined): string[] {
  return (urls ?? []).map((url) => url.trim()).filter((url) => url.length > 0)
}

// --- dual-stack value helpers (pure, shared by forms and tests) -------------

function ipv6GroupsFromParts(parts: string[]): number[] | null {
  const groups: number[] = []
  for (const part of parts) {
    if (part.includes('.')) {
      // Embedded IPv4 (e.g. ::ffff:192.0.2.1) counts as two groups.
      if (!/^(?:0|[1-9]\d{0,2})(?:\.(?:0|[1-9]\d{0,2})){3}$/.test(part)) return null
      const v4 = part.split('.').map(Number)
      if (v4.length !== 4 || v4.some(n => !Number.isInteger(n) || n < 0 || n > 255)) {
        return null
      }
      groups.push((v4[0] << 8) | v4[1], (v4[2] << 8) | v4[3])
      continue
    }
    if (!/^[0-9a-fA-F]{1,4}$/.test(part)) {
      return null
    }
    groups.push(Number.parseInt(part, 16))
  }
  return groups
}

/** Parses an IPv6 address into 8 numeric 16-bit groups, or null. */
export function parseIpv6Groups(addr: string): number[] | null {
  const s = addr.trim()
  if (!s.includes(':')) {
    return null
  }
  if (s.includes('.')) {
    const parts = s.split(':')
    // An embedded IPv4 literal can only occupy the final 32 address bits.
    if (!parts[parts.length - 1]?.includes('.') || parts.filter(part => part.includes('.')).length !== 1) return null
  }
  const doubleColonCount = (s.match(/::/g) ?? []).length
  if (doubleColonCount > 1) {
    return null
  }

  let headParts: string[] = []
  let tailParts: string[] = []
  if (doubleColonCount === 1) {
    const sides = s.split('::')
    headParts = sides[0].length ? sides[0].split(':') : []
    tailParts = sides[1].length ? sides[1].split(':') : []
  } else {
    headParts = s.split(':')
  }

  const headGroups = ipv6GroupsFromParts(headParts)
  const tailGroups = ipv6GroupsFromParts(tailParts)
  if (!headGroups || !tailGroups) {
    return null
  }

  if (doubleColonCount === 0) {
    return headGroups.length === 8 ? headGroups : null
  }
  if (headGroups.length + tailGroups.length > 7) {
    // "::" must stand for at least one zero group.
    return null
  }
  const zeros = new Array<number>(8 - headGroups.length - tailGroups.length).fill(0)
  return [...headGroups, ...zeros, ...tailGroups]
}

/**
 * Validates an IPv6 CIDR ("addr/prefix", prefix 0-128). Rejects IPv4.
 * Host bits are intentionally not inspected: node addresses keep them.
 */
export function isValidIpv6Cidr(value: string): boolean {
  const trimmed = value.trim()
  const slash = trimmed.lastIndexOf('/')
  if (slash === -1) {
    return false
  }
  const addr = trimmed.slice(0, slash)
  const prefixStr = trimmed.slice(slash + 1)
  if (!/^\d{1,3}$/.test(prefixStr)) {
    return false
  }
  const prefix = Number.parseInt(prefixStr, 10)
  if (prefix < 0 || prefix > 128) {
    return false
  }
  return parseIpv6Groups(addr) !== null
}

/**
 * Normalizes a network-route IPv6 CIDR to its network address (host bits
 * cleared). Returns null for invalid input. Node address fields must NOT use
 * this — they keep host bits.
 */
export function normalizeIpv6RouteCidr(value: string): string | null {
  const trimmed = value.trim()
  if (!isValidIpv6Cidr(trimmed)) {
    return null
  }
  const slash = trimmed.lastIndexOf('/')
  const groups = parseIpv6Groups(trimmed.slice(0, slash))!
  const prefix = Number.parseInt(trimmed.slice(slash + 1), 10)
  for (let g = 0; g < 8; g++) {
    const bitsIntoGroup = prefix - g * 16
    if (bitsIntoGroup <= 0) {
      groups[g] = 0
    } else if (bitsIntoGroup < 16) {
      groups[g] &= (0xffff << (16 - bitsIntoGroup)) & 0xffff
    }
  }
  return `${groups.map(g => g.toString(16)).join(':')}/${prefix}`
}

/** Single IPv4 or IPv6 literal (no port, no prefix). */
export function isValidIpLiteral(value: string): boolean {
  const trimmed = value.trim()
  if (!trimmed) {
    return false
  }
  if (trimmed.includes(':')) {
    return parseIpv6Groups(trimmed) !== null
  }
  if (!/^(?:0|[1-9]\d{0,2})(?:\.(?:0|[1-9]\d{0,2})){3}$/.test(trimmed)) return false
  const v4 = trimmed.split('.').map(Number)
  return v4.length === 4 && v4.every(n => Number.isInteger(n) && n >= 0 && n <= 255)
}


export function normalizeNetworkConfig(config: NetworkConfig): NetworkConfig {
  const normalized: NetworkConfig = {
    ...config,
    peer_urls: cleanPeerUrls(config.peer_urls),
    // Missing array fields default to empty arrays; other known values pass
    // through untouched via the spread above.
    routes: config.routes ?? [],
    routes_v6: config.routes_v6 ?? [],
  }

  const publicServerUrl = normalized.public_server_url?.trim() ?? ''

  switch (normalized.networking_method) {
    case NetworkingMethod.PublicServer:
      normalized.peer_urls = publicServerUrl ? [publicServerUrl] : []
      break
    case NetworkingMethod.Manual:
      break
    case NetworkingMethod.Standalone:
    default:
      normalized.peer_urls = []
      break
  }

  normalized.networking_method = NetworkingMethod.Manual
  normalized.public_server_url = ''
  return normalized
}

export function toBackendNetworkConfig(config: NetworkConfig): NetworkConfig {
  return normalizeNetworkConfig(config)
}

export interface NetworkInstance {
  instance_id: string

  running: boolean
  error_msg: string

  detail?: NetworkInstanceRunningInfo
}

export interface NetworkInstanceRunningInfo {
  dev_name: string
  my_node_info: NodeInfo
  events: Array<string>,
  routes: Route[]
  peers: PeerInfo[]
  peer_route_pairs: PeerRoutePair[]
  running: boolean
  error_msg?: string
}

export interface Ipv4Addr {
  addr: number
}

export interface Ipv4Inet {
  address: Ipv4Addr
  network_length: number
}

export interface Ipv6Addr {
  part1: number
  part2: number
  part3: number
  part4: number
}

export interface Url {
  url: string
}

export interface NodeInfo {
  virtual_ipv4: Ipv4Inet,
  hostname: string
  version: string
  ips: {
    public_ipv4: Ipv4Addr
    interface_ipv4s: Ipv4Addr[]
    public_ipv6: Ipv6Addr
    interface_ipv6s: Ipv6Addr[]
    listeners: {
      serialization: string
      scheme_end: number
      username_end: number
      host_start: number
      host_end: number
      host: any
      port?: number
      path_start: number
      query_start?: number
      fragment_start?: number
    }[]
  }
  stun_info: StunInfo
  listeners: Url[]
  vpn_portal_cfg?: string
  peer_id: number
}

export interface StunInfo {
  udp_nat_type: number
  tcp_nat_type: number
  last_update_time: number
}

export interface Route {
  peer_id: number
  ipv4_addr: Ipv4Inet | string | null
  next_hop_peer_id: number
  cost: number
  proxy_cidrs: string[]
  hostname: string
  stun_info?: StunInfo
  inst_id: string
  version: string
}

export interface PeerInfo {
  peer_id: number
  conns: PeerConnInfo[]
}

export interface PeerConnInfo {
  conn_id: string
  my_peer_id: number
  is_client: boolean
  peer_id: number
  features: string[]
  tunnel?: TunnelInfo
  stats?: PeerConnStats
  loss_rate: number
}

export interface PeerRoutePair {
  route: Route
  peer?: PeerInfo
}

export interface UrlPb {
  url: string
}

export interface TunnelInfo {
  tunnel_type: string
  local_addr: UrlPb
  remote_addr: UrlPb
}

export interface PeerConnStats {
  rx_bytes: number
  tx_bytes: number
  rx_packets: number
  tx_packets: number
  latency_us: number
}

export interface PortForwardConfig {
  bind_ip: string,
  bind_port: number,
  dst_ip: string,
  dst_port: number,
  proto: string
}

// 添加新行
export const addRow = (rows: PortForwardConfig[]) => {
  rows.push({
    proto: 'tcp',
    bind_ip: '',
    bind_port: 65535,
    dst_ip: '',
    dst_port: 65535,
  });
};

// 删除行
export const removeRow = (index: number, rows: PortForwardConfig[]) => {
  rows.splice(index, 1);
};

export enum EventType {
  TunDeviceReady = 'TunDeviceReady', // string
  TunDeviceError = 'TunDeviceError', // string

  PeerAdded = 'PeerAdded', // number
  PeerRemoved = 'PeerRemoved', // number
  PeerConnAdded = 'PeerConnAdded', // PeerConnInfo
  PeerConnRemoved = 'PeerConnRemoved', // PeerConnInfo

  ListenerAdded = 'ListenerAdded', // any
  ListenerAddFailed = 'ListenerAddFailed', // any, string
  ListenerAcceptFailed = 'ListenerAcceptFailed', // any, string
  ConnectionAccepted = 'ConnectionAccepted', // string, string
  ConnectionError = 'ConnectionError', // string, string, string

  Connecting = 'Connecting', // any
  ConnectError = 'ConnectError', // string, string, string

  VpnPortalStarted = 'VpnPortalStarted', // string
  VpnPortalClientConnected = 'VpnPortalClientConnected', // string, string
  VpnPortalClientDisconnected = 'VpnPortalClientDisconnected', // string, string, string

  DhcpIpv4Changed = 'DhcpIpv4Changed', // ipv4 | null, ipv4 | null
  DhcpIpv4Conflicted = 'DhcpIpv4Conflicted', // ipv4 | null

  PortForwardAdded = 'PortForwardAdded', // PortForwardConfigPb

  ProxyCidrsUpdated = 'ProxyCidrsUpdated', // string[], string[]

  UdpBroadcastRelayStartResult = 'UdpBroadcastRelayStartResult', // { capture_backend?: string, error?: string }
}
