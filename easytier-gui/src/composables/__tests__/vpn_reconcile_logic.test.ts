import { describe, expect, it } from 'vitest'
import {
  netParamsChanged,
  routingSignature,
  shouldRebuildVpn,
} from '../vpn_reconcile_logic'

describe('routingSignature', () => {
  it('compares current UIDs, including reinstall changes and shared-UID members', () => {
    const before = routingSignature({ mode: 'include', packages: ['a.app'], uids: [100] })
    expect(before).not.toBe(routingSignature({ mode: 'include', packages: ['a.app'], uids: [101] }))
    expect(before).toBe(routingSignature({ mode: 'include', packages: ['a.app', 'shared.app'], uids: [100, 100] }))
  })
  it('is invariant under reordering and duplicates', () => {
    const a = routingSignature({ mode: 'include', packages: ['b.app', 'a.app', 'b.app'] })
    const b = routingSignature({ mode: 'include', packages: ['a.app', 'b.app'] })
    expect(a).toBe(b)
  })

  it('distinguishes modes and package sets', () => {
    const include = routingSignature({ mode: 'include', packages: ['a.app'] })
    const all = routingSignature({ mode: 'all', packages: [] })
    const includeMore = routingSignature({ mode: 'include', packages: ['a.app', 'c.app'] })
    expect(include).not.toBe(all)
    expect(include).not.toBe(includeMore)
  })

  it('maps unknown/legacy state to a signature that mismatches everything', () => {
    const unknown = routingSignature({ mode: null, packages: ['a.app'] })
    expect(unknown).toBe('unknown')
    expect(unknown).not.toBe(routingSignature({ mode: 'all', packages: [] }))
  })
})

describe('netParamsChanged', () => {
  const base = {
    ipv4Addr: '10.1.1.2',
    ipv4Cidr: 24,
    ipv6Addr: 'fd79::8/64',
    routes: ['0.0.0.0/1', '128.0.0.0/1'],
    dns: '10.1.1.1',
  }

  it('ignores route order changes', () => {
    expect(netParamsChanged(base, { ...base, routes: [...base.routes].reverse() })).toBe(false)
  })

  it('detects address, cidr, v6, dns and route-set changes', () => {
    expect(netParamsChanged(base, { ...base, ipv4Addr: '10.1.1.3' })).toBe(true)
    expect(netParamsChanged(base, { ...base, ipv4Cidr: 32 })).toBe(true)
    expect(netParamsChanged(base, { ...base, ipv6Addr: undefined })).toBe(true)
    expect(netParamsChanged(base, { ...base, dns: undefined })).toBe(true)
    expect(netParamsChanged(base, { ...base, routes: ['0.0.0.0/0'] })).toBe(true)
  })
})

describe('shouldRebuildVpn', () => {
  const applied = 'include|a.app,b.app'
  const desired = 'include|a.app,b.app'

  it('never rebuilds while not running', () => {
    expect(shouldRebuildVpn({
      running: false,
      netChanged: true,
      appliedSignature: 'unknown',
      desiredSignature: desired,
    })).toBe(false)
  })

  it('keeps a running VPN when nothing changed', () => {
    expect(shouldRebuildVpn({
      running: true,
      netChanged: false,
      appliedSignature: applied,
      desiredSignature: desired,
    })).toBe(false)
  })

  it('rebuilds once for a legacy applied state (unknown signature)', () => {
    const result = shouldRebuildVpn({
      running: true,
      netChanged: false,
      appliedSignature: 'unknown',
      desiredSignature: desired,
    })
    expect(result).toBe(true)
    // After that rebuild the applied signature matches and it settles.
    expect(shouldRebuildVpn({
      running: true,
      netChanged: false,
      appliedSignature: desired,
      desiredSignature: desired,
    })).toBe(false)
  })

  it('rebuilds on network param changes even with identical app lists', () => {
    expect(shouldRebuildVpn({
      running: true,
      netChanged: true,
      appliedSignature: applied,
      desiredSignature: desired,
    })).toBe(true)
  })
})
