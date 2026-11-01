import { describe, expect, it } from 'vitest'
import { NetworkConfig as NetworkConfigPb } from '../src/generated/proto/api_manage'
import { normalizeNetworkConfig, toBackendNetworkConfig } from '../src/types/network'

describe('custom dual-stack config migration', () => {
  it('preserves existing JSON fields through form and protobuf conversion', () => {
    const legacy: any = {
      virtual_ipv6: 'fd12:3456::8/64',
      vpn_dns: '192.0.2.53',
      routes_v6: ['::/1', '8000::/1'],
      exit_nodes: ['192.0.2.1', 'fd12:3456::1'],
    }
    const saved: any = toBackendNetworkConfig(normalizeNetworkConfig(legacy))
    expect(saved.virtual_ipv6).toBe(legacy.virtual_ipv6)
    expect(saved.vpn_dns).toBe(legacy.vpn_dns)
    expect(saved.routes_v6).toEqual(legacy.routes_v6)
    expect(saved.exit_nodes).toEqual(legacy.exit_nodes)
    expect(saved.enable_manual_routes).not.toBe(true)
    const restored = NetworkConfigPb.fromBinary(NetworkConfigPb.toBinary(
      NetworkConfigPb.fromJson(saved),
    ))
    expect(NetworkConfigPb.toJson(restored, { useProtoFieldName: true })).toEqual(saved)
  })

  it('preserves an explicitly empty IPv6 manual list independently of IPv4', () => {
    const saved: any = toBackendNetworkConfig(normalizeNetworkConfig({
      enable_manual_routes_v6: true,
      routes_v6: [],
    } as any))
    expect(saved.enable_manual_routes_v6).toBe(true)
    expect(saved.routes_v6 ?? []).toEqual([])
    expect(saved.enable_manual_routes).not.toBe(true)
  })
})
