import { describe, expect, it } from 'vitest'
import {
  DEFAULT_NETWORK_CONFIG,
  isValidIpLiteral,
  isValidIpv6Cidr,
  normalizeIpv6RouteCidr,
  normalizeNetworkConfig,
} from '../../../easytier-web/frontend-lib/src/types/network'

describe('isValidIpv6Cidr', () => {
  it('accepts full and compressed forms with prefix 0-128', () => {
    expect(isValidIpv6Cidr('fd79:68a2:532c::8/64')).toBe(true)
    expect(isValidIpv6Cidr('::/0')).toBe(true)
    expect(isValidIpv6Cidr('2001:db8::1/128')).toBe(true)
    expect(isValidIpv6Cidr('::ffff:192.0.2.1/96')).toBe(true)
  })

  it('rejects missing prefix, out-of-range prefix and IPv4 input', () => {
    expect(isValidIpv6Cidr('fd79::8')).toBe(false)
    expect(isValidIpv6Cidr('fd79::8/129')).toBe(false)
    expect(isValidIpv6Cidr('fd79::8/abc')).toBe(false)
    expect(isValidIpv6Cidr('192.168.0.0/16')).toBe(false)
    expect(isValidIpv6Cidr('fd79::::8/64')).toBe(false)
    expect(isValidIpv6Cidr('1:2:3:4:5:6:7:8:9/64')).toBe(false)
  })
})

describe('normalizeIpv6RouteCidr', () => {
  it('clears host bits for network routes', () => {
    expect(normalizeIpv6RouteCidr('fd79:68a2:532c:1:ffff:ffff:ffff:ffff/48'))
      .toBe('fd79:68a2:532c:0:0:0:0:0/48')
    expect(normalizeIpv6RouteCidr('fd00::1/8')).toBe('fd00:0:0:0:0:0:0:0/8')
  })

  it('returns null for invalid input', () => {
    expect(normalizeIpv6RouteCidr('not-a-cidr')).toBeNull()
    expect(normalizeIpv6RouteCidr('10.0.0.0/8')).toBeNull()
  })
})

describe('isValidIpLiteral', () => {
  it('accepts single IPv4/IPv6 literals without port or prefix', () => {
    expect(isValidIpLiteral('10.126.144.1')).toBe(true)
    expect(isValidIpLiteral('fd79::1')).toBe(true)
  })

  it('rejects CIDRs, ports and garbage', () => {
    expect(isValidIpLiteral('10.126.144.1/24')).toBe(false)
    expect(isValidIpLiteral('10.126.144.1:53')).toBe(false)
    expect(isValidIpLiteral('')).toBe(false)
    expect(isValidIpLiteral('abc')).toBe(false)
  })

  it('rejects malformed numeric IPv4 and misplaced embedded IPv4', () => {
    for (const value of ['.1.2.3', '1e2.1.2.3', '01.1.2.3', '1.2.3.4::', '::1.2.3.4:1', '1.2.3.4::5.6.7.8']) {
      expect(isValidIpLiteral(value)).toBe(false)
    }
    expect(isValidIpv6Cidr('1.2.3.4::/64')).toBe(false)
    expect(isValidIpLiteral('::ffff:192.0.2.1')).toBe(true)
  })
})

describe('normalizeNetworkConfig dual-stack fields', () => {
  it('fills missing routes arrays without dropping known values', () => {
    const input = {
      ...DEFAULT_NETWORK_CONFIG(),
      routes: undefined as unknown as string[],
      routes_v6: undefined as unknown as string[],
      virtual_ipv6: 'fd79:68a2:532c::8/64',
      vpn_dns: '10.126.144.1',
      exit_nodes: ['10.126.144.1'],
    }
    const normalized = normalizeNetworkConfig(input)
    expect(normalized.routes).toEqual([])
    expect(normalized.routes_v6).toEqual([])
    expect(normalized.virtual_ipv6).toBe('fd79:68a2:532c::8/64')
    expect(normalized.vpn_dns).toBe('10.126.144.1')
    expect(normalized.exit_nodes).toEqual(['10.126.144.1'])
  })
})
