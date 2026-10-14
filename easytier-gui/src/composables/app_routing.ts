import {
  get_app_routing_policy,
  set_app_routing_policy,
  get_installed_apps,
  get_app_icon,
  type AppCatalogResponse,
  type AppRoutingPolicy,
} from 'tauri-plugin-vpnservice-api'

/**
 * Per-app routing page support: policy initialization/migration and the
 * catalog/icon caches. This module never starts or stops the VPN itself;
 * rebuilds go through mobile_vpn's serial reconcile entry.
 */

let policyInitialized = false
let policyInitPromise: Promise<void> | null = null

function legacyNetworkListExists(): boolean {
  try {
    const raw = localStorage.getItem('networkList')
    if (!raw) {
      return false
    }
    const parsed = JSON.parse(raw)
    return Array.isArray(parsed) && parsed.length > 0
  } catch {
    return false
  }
}

/**
 * One-time policy initialization, required before any VPN start:
 *  - policy missing + legacy networkList present  -> save "all" (keeps the
 *    previous all-apps behavior across the upgrade)
 *  - policy missing + fresh install               -> save "include" + []
 *  - policy present (valid or throwing)           -> untouched
 */
export async function ensureAppRoutingPolicyInitialized(): Promise<void> {
  if (policyInitialized) {
    return
  }
  if (!policyInitPromise) {
    policyInitPromise = (async () => {
      const resp = await get_app_routing_policy()
      if (resp.policy === null) {
        const policy: AppRoutingPolicy = legacyNetworkListExists()
          ? { schemaVersion: 1, mode: 'all', packages: [] }
          : { schemaVersion: 1, mode: 'include', packages: [] }
        // validateOnly=false: saving include+[] is legal (it only refuses to
        // start until apps are selected).
        await set_app_routing_policy(policy, false)
      }
      policyInitialized = true
    })().catch((e) => {
      policyInitPromise = null
      throw e
    })
  }
  return policyInitPromise
}

/** Test hook. */
export function resetAppRoutingPolicyInit(): void {
  policyInitialized = false
  policyInitPromise = null
}

export type { AppCatalogResponse }

export async function fetchAppCatalog(): Promise<AppCatalogResponse> {
  return await get_installed_apps()
}

export async function fetchAppIcon(packageName: string): Promise<string | null> {
  const resp = await get_app_icon(packageName)
  return resp.dataUrl ?? null
}

// In-memory icon cache keyed by iconKey (packageName@lastUpdateTime). Never
// persisted; dropped with the page.
const iconCache = new Map<string, string | null>()
const iconInFlight = new Map<string, Promise<string | null>>()
let cacheGeneration = 0

export function cachedIcon(iconKey: string): string | null | undefined {
  return iconCache.get(iconKey)
}

export function loadIcon(iconKey: string, packageName: string): Promise<string | null> {
  const cached = iconCache.get(iconKey)
  if (cached !== undefined) {
    return Promise.resolve(cached)
  }
  const inFlight = iconInFlight.get(iconKey)
  if (inFlight) {
    return inFlight
  }
  const generation = cacheGeneration
  const promise = fetchAppIcon(packageName)
    .then((dataUrl) => {
      if (generation === cacheGeneration) iconCache.set(iconKey, dataUrl)
      return dataUrl
    })
    .catch((e) => {
      console.warn('app icon load failed', packageName, e)
      // Remember the failure so a missing icon never retries in a loop.
      if (generation === cacheGeneration) iconCache.set(iconKey, null)
      return null
    })
    .finally(() => {
      iconInFlight.delete(iconKey)
    })
  iconInFlight.set(iconKey, promise)
  return promise
}

export function dropIconCache(): void {
  cacheGeneration++
  iconCache.clear()
}
