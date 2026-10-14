import { beforeEach, describe, expect, it, vi } from 'vitest'

const mocks = vi.hoisted(() => ({
  handlers: new Map<string, (payload: any) => Promise<void>>(),
  policy: { schemaVersion: 1, mode: 'include' as 'include' | 'all', packages: ['a.app'] },
  runningIds: ['net'],
  nativeRunning: false,
  failAttach: false,
  start: vi.fn(), stop: vi.fn(), tun: vi.fn(), save: vi.fn(),
  stopHook: undefined as (() => Promise<void>) | undefined,
}))
vi.mock('@tauri-apps/api/core', () => ({
  addPluginListener: vi.fn(async (_plugin, event, fn) => { mocks.handlers.set(event, fn) }),
}))
vi.mock('easytier-frontend-lib', () => ({ Utils: {
  UuidToStr: (id: string) => id,
  ipv4ToString: () => '10.1.1.2',
} }))
vi.mock('../app_routing', () => ({ ensureAppRoutingPolicyInitialized: vi.fn(async () => {}) }))
vi.mock('../backend', () => ({
  listNetworkInstanceIds: vi.fn(async () => ({ running_inst_ids: [...mocks.runningIds] })),
  getConfig: vi.fn(async () => ({ routes: ['0.0.0.0/0'], routes_v6: ['::/0'], virtual_ipv6: 'fd79::8/64', vpn_dns: '10.1.1.1' })),
  collectNetworkInfo: vi.fn(async () => ({ info: { map: { net: {
    my_node_info: { virtual_ipv4: { address: {}, network_length: 24 } }, routes: [],
  } } } })),
  setTunFd: (...args: any[]) => mocks.tun(...args),
}))
vi.mock('tauri-plugin-vpnservice-api', () => {
  const effective = (policy: typeof mocks.policy) => ({
    mode: policy.mode,
    packages: policy.mode === 'all' ? [] : policy.packages.filter(pkg => pkg !== 'gone.app'),
    uids: policy.mode === 'all' ? [] : policy.packages.filter(pkg => pkg !== 'gone.app').map(pkg => pkg === 'a.app' ? 100 : 101),
  })
  return {
    prepare_vpn: vi.fn(async () => ({ granted: true })),
    get_vpn_status: vi.fn(async () => ({ running: mocks.nativeRunning })),
    get_app_routing_policy: vi.fn(async () => ({ policy: structuredClone(mocks.policy), effectivePolicy: effective(mocks.policy) })),
    set_app_routing_policy: async (policy: typeof mocks.policy, validateOnly: boolean) => {
      if (validateOnly && policy.mode === 'include' && !effective(policy).uids.length) throw new Error('no_effective_apps')
      if (!validateOnly) { mocks.save(policy); mocks.policy = structuredClone(policy) }
      return { policy, effectivePolicy: effective(policy), skippedPackages: [] }
    },
    start_vpn: async (request: any) => {
      mocks.start(request)
      mocks.nativeRunning = true
      await mocks.handlers.get('vpn_service_start')!({ fd: 7, requestId: request.requestId, effectivePolicy: effective(mocks.policy), skippedPackages: [] })
      return {}
    },
    stop_vpn: async (requestId: string) => {
      mocks.stop(requestId)
      if (mocks.stopHook) await mocks.stopHook()
      mocks.nativeRunning = false
      await mocks.handlers.get('vpn_service_stop')!({ requestId })
      return {}
    },
  }
})

let vpn: typeof import('../mobile_vpn')
beforeEach(async () => {
  vi.resetModules()
  mocks.handlers.clear()
  mocks.policy = { schemaVersion: 1, mode: 'include', packages: ['a.app'] }
  mocks.runningIds = ['net']; mocks.nativeRunning = false; mocks.failAttach = false; mocks.stopHook = undefined
  mocks.start.mockReset(); mocks.stop.mockReset(); mocks.save.mockReset()
  mocks.tun.mockReset().mockImplementation(async (fd: number) => {
    if (fd > 0 && mocks.failAttach) { mocks.failAttach = false; throw new Error('attach_failed') }
  })
  vpn = await import('../mobile_vpn')
  await vpn.initMobileVpnService()
})
async function boot() {
  await vpn.onNetworkInstanceChange('net')
  // Applying an unchanged policy queues behind startup and waits for its ack.
  await vpn.applyAppRoutingPolicy(structuredClone(mocks.policy))
  expect(mocks.start).toHaveBeenCalledTimes(1)
  expect(mocks.stop).not.toHaveBeenCalled()
}
describe('mobile VPN coordinator', () => {
  it('saves while stopped without starting a VPN', async () => {
    expect(await vpn.applyAppRoutingPolicy({ schemaVersion: 1, mode: 'include', packages: ['b.app'] })).toEqual({ status: 'saved' })
    expect(mocks.start).not.toHaveBeenCalled()
  })
  it('rebuilds once for a changed UID set and detaches before closing', async () => {
    await boot()
    expect((await vpn.applyAppRoutingPolicy({ schemaVersion: 1, mode: 'include', packages: ['b.app'] })).status).toBe('applied')
    expect(mocks.start).toHaveBeenCalledTimes(2)
    expect(mocks.stop).toHaveBeenCalledTimes(1)
    const detachIndex = mocks.tun.mock.calls.findIndex(([fd]) => fd === -1)
    expect(mocks.tun.mock.invocationCallOrder[detachIndex]).toBeLessThan(mocks.stop.mock.invocationCallOrder[0])
  })
  it('refuses an empty effective list without touching the old VPN', async () => {
    await boot()
    await expect(vpn.applyAppRoutingPolicy({ schemaVersion: 1, mode: 'include', packages: ['gone.app'] })).rejects.toThrow('no_effective_apps')
    expect(mocks.stop).not.toHaveBeenCalled()
    expect(mocks.policy.packages).toEqual(['a.app'])
  })
  it('does not rebuild for persisted unavailable packages outside the effective set', async () => {
    await boot()
    await vpn.applyAppRoutingPolicy({ schemaVersion: 1, mode: 'include', packages: ['a.app', 'gone.app'] })
    expect(mocks.stop).not.toHaveBeenCalled()
    expect(mocks.policy.packages).toEqual(['a.app', 'gone.app'])
  })
  it('cleans a failed attach and restores the previous policy once', async () => {
    await boot()
    mocks.failAttach = true
    expect((await vpn.applyAppRoutingPolicy({ schemaVersion: 1, mode: 'include', packages: ['b.app'] })).status).toBe('restored')
    expect(mocks.policy.packages).toEqual(['a.app'])
    expect(mocks.start).toHaveBeenCalledTimes(3)
    expect(mocks.nativeRunning).toBe(true)
  })
  it('keeps the new selection without resurrecting a user-deleted network', async () => {
    await boot()
    mocks.stopHook = async () => { mocks.runningIds = []; await vpn.onNetworkInstanceChange('net') }
    expect((await vpn.applyAppRoutingPolicy({ schemaVersion: 1, mode: 'include', packages: ['b.app'] })).status).toBe('saved')
    expect(mocks.start).toHaveBeenCalledTimes(1)
    expect(mocks.policy.packages).toEqual(['b.app'])
  })
  it('ignores late start events from previous request IDs', async () => {
    await boot()
    await mocks.handlers.get('vpn_service_start')!({ fd: 99, requestId: 'old-request' })
    expect(mocks.tun.mock.calls.some(([fd]) => fd === 99)).toBe(false)
  })
})
