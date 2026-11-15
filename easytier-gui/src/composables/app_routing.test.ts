import { beforeEach, describe, expect, it, vi } from 'vitest'

const mocks = vi.hoisted(() => ({
  get: vi.fn(),
  set: vi.fn(async () => ({})),
  storage: vi.fn(),
}))
vi.mock('tauri-plugin-vpnservice-api', () => ({
  get_app_routing_policy: mocks.get,
  set_app_routing_policy: mocks.set,
  get_installed_apps: vi.fn(),
  get_app_icon: vi.fn(),
}))

beforeEach(() => {
  vi.resetModules()
  vi.clearAllMocks()
  mocks.get.mockResolvedValue({ policy: null })
  mocks.storage.mockReturnValue(null)
  vi.stubGlobal('localStorage', { getItem: mocks.storage })
})

describe('application policy initialization', () => {
  it('initializes a fresh install once before its first network is created', async () => {
    const { ensureAppRoutingPolicyInitialized } = await import('./app_routing')
    await Promise.all([ensureAppRoutingPolicyInitialized(), ensureAppRoutingPolicyInitialized()])
    mocks.storage.mockReturnValue('[{"id":"new-network"}]')
    await ensureAppRoutingPolicyInitialized()
    expect(mocks.set).toHaveBeenCalledOnce()
    expect(mocks.set).toHaveBeenCalledWith({ schemaVersion: 1, mode: 'include', packages: [] }, false)
  })

  it('preserves all-app routing when upgrading a legacy installation', async () => {
    mocks.storage.mockReturnValue('[{"id":"legacy-network"}]')
    const { ensureAppRoutingPolicyInitialized } = await import('./app_routing')
    await ensureAppRoutingPolicyInitialized()
    expect(mocks.set).toHaveBeenCalledOnce()
    expect(mocks.set).toHaveBeenCalledWith({ schemaVersion: 1, mode: 'all', packages: [] }, false)
  })

  it('retains a saved policy and does not replace an unreadable policy', async () => {
    mocks.get.mockResolvedValueOnce({ policy: { schemaVersion: 1, mode: 'include', packages: ['app.a'] } })
    const { ensureAppRoutingPolicyInitialized, resetAppRoutingPolicyInit } = await import('./app_routing')
    await ensureAppRoutingPolicyInitialized()
    resetAppRoutingPolicyInit()
    mocks.get.mockRejectedValueOnce(new Error('invalid stored policy'))
    await expect(ensureAppRoutingPolicyInitialized()).rejects.toThrow('invalid stored policy')
    expect(mocks.set).not.toHaveBeenCalled()
  })
})
