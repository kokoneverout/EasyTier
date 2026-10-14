/**
 * Pure comparison helpers for the mobile VPN serial reconcile loop.
 *
 * Kept free of Tauri/native imports so the rebuild decisions (what counts as
 * a change, when the interface must be rebuilt, what is intentionally ignored)
 * stay unit-testable.
 */

export interface VpnNetParams {
  ipv4Addr: string | undefined
  ipv4Cidr: number | undefined
  ipv6Addr: string | undefined
  routes: string[]
  dns: string | undefined
}

export interface AppliedRoutingState {
  /** Mode of the actually-established interface; null when unknown (legacy). */
  mode: string | null | undefined
  packages: string[] | undefined
  uids?: number[]
}

export function netParamsChanged(applied: VpnNetParams, desired: VpnNetParams): boolean {
  return (
    applied.ipv4Addr !== desired.ipv4Addr
    || applied.ipv4Cidr !== desired.ipv4Cidr
    || applied.ipv6Addr !== desired.ipv6Addr
    || applied.dns !== desired.dns
    || stringifyRoutes(applied.routes) !== stringifyRoutes(desired.routes)
  )
}

function stringifyRoutes(routes: string[] | undefined): string {
  return JSON.stringify([...(routes ?? [])].sort())
}

/**
 * Canonical signature of an app routing configuration: mode plus the sorted,
 * deduped effective package list. Order-only changes (re-sorting, icon
 * arrival) never produce a different signature.
 * Unknown/legacy applied mode maps to "unknown" so it mismatches everything
 * and forces exactly one rebuild.
 */
export function routingSignature(state: AppliedRoutingState | undefined | null): string {
  if (!state?.mode) {
    return 'unknown'
  }
  if (state.mode === 'all') return 'all'
  if (state.uids) return `${state.mode}|uids:${[...new Set(state.uids)].sort((a, b) => a - b).join(',')}`
  // Legacy interfaces without UID state are rebuilt once by the coordinator.
  return `${state.mode}|packages:${[...new Set(state.packages ?? [])].sort().join(',')}`
}

export function shouldRebuildVpn(opts: {
  running: boolean
  netChanged: boolean
  appliedSignature: string
  desiredSignature: string
}): boolean {
  if (!opts.running) {
    // Not running: the next start applies everything; nothing to rebuild.
    return false
  }
  return opts.netChanged || opts.appliedSignature !== opts.desiredSignature
}
