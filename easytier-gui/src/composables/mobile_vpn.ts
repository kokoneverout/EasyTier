import { ref } from 'vue'
import type { NetworkTypes } from 'easytier-frontend-lib'
import { addPluginListener } from '@tauri-apps/api/core'
import { Utils } from 'easytier-frontend-lib'
import {
  get_vpn_status,
  prepare_vpn,
  start_vpn,
  stop_vpn,
  get_app_routing_policy,
  set_app_routing_policy,
  type AppRoutingPolicy,
  type EffectiveAppRoutingPolicy,
  type SkippedPackage,
  type StartVpnRequest,
} from 'tauri-plugin-vpnservice-api'
import { getConfig, collectNetworkInfo, listNetworkInstanceIds, setTunFd } from './backend'
import { ensureAppRoutingPolicyInitialized } from './app_routing'
import {
  netParamsChanged,
  routingSignature,
  shouldRebuildVpn,
  type AppliedRoutingState,
} from './vpn_reconcile_logic'

type Route = NetworkTypes.Route

interface vpnStatus {
  running: boolean
  ipv4Addr: string | null | undefined
  ipv4Cidr: number | null | undefined
  ipv6Addr: string | null | undefined
  routes: string[]
  dns: string | null | undefined
  /** Mode of the actually-established interface; null = unknown (legacy install). */
  applicationMode: 'include' | 'all' | null
  /** Effective package set the established interface captures, sorted. */
  appliedPackages: string[] | undefined
  appliedUids: number[] | undefined
  skippedPackages: SkippedPackage[]
  /** Instance the current fd was attached to; used for explicit detach. */
  attachedInstanceId: string | undefined
}

let dhcpPollingTimer: NodeJS.Timeout | null = null
const DHCP_POLLING_INTERVAL = 2000

const curVpnStatus: vpnStatus = {
  running: false,
  ipv4Addr: undefined,
  ipv4Cidr: undefined,
  ipv6Addr: undefined,
  routes: [],
  dns: undefined,
  applicationMode: null,
  appliedPackages: undefined,
  appliedUids: undefined,
  skippedPackages: [],
  attachedInstanceId: undefined,
}

// ---------------------------------------------------------------------------
// Serial operation queue: every VPN stop/start/rebuild and policy apply runs
// here, one at a time. Network events arriving mid-operation only schedule a
// single follow-up reconcile that re-reads the latest state.
// ---------------------------------------------------------------------------

let vpnOpChain: Promise<unknown> = Promise.resolve()

function enqueueVpnOperation<T>(op: () => Promise<T>): Promise<T> {
  const run = vpnOpChain.then(op, op)
  vpnOpChain = run.catch(() => undefined)
  return run
}

let reconcileQueued = false
let reconcileRunning = false
let desiredInstanceId: string | undefined
export const vpnServiceError = ref<string | null>(null)

function scheduleReconcile(trigger: string) {
  console.log('vpn service reconcile scheduled', trigger)
  if (reconcileRunning) {
    // One follow-up pass after the in-flight one; it will read the then-current
    // network params and saved policy, never a stale snapshot.
    reconcileQueued = true
    return
  }
  reconcileRunning = true
  void enqueueVpnOperation(async () => {
    try {
      do {
        reconcileQueued = false
        await reconcileOnce()
      } while (reconcileQueued)
    } catch (e) {
      vpnServiceError.value = errorMessage(e)
      console.error('vpn service reconcile failed', e)
    } finally {
      reconcileRunning = false
      if (reconcileQueued) scheduleReconcile('pending_after_failure')
    }
  })
}

// ---------------------------------------------------------------------------
// Start/stop completion plumbing driven by native events (not polling).
// ---------------------------------------------------------------------------

interface StartResultPayload {
  fd?: number
  requestId?: string
  effectivePolicy?: EffectiveAppRoutingPolicy
  skippedPackages?: SkippedPackage[]
}

interface StartWaiter {
  requestId: string
  resolve: (payload: StartResultPayload) => void
  reject: (err: Error) => void
  timer: NodeJS.Timeout
}

let activeStartWaiter: StartWaiter | null = null
const START_RESULT_TIMEOUT_MS = 10000

function expectStartResult(requestId: string): Promise<StartResultPayload> {
  clearStartWaiter()
  return new Promise<StartResultPayload>((resolve, reject) => {
    const timer = setTimeout(() => {
      if (activeStartWaiter?.requestId === requestId) {
        activeStartWaiter = null
      }
      reject(new Error('vpn_start_timeout'))
    }, START_RESULT_TIMEOUT_MS)
    activeStartWaiter = { requestId, resolve, reject, timer }
  })
}

function clearStartWaiter() {
  if (activeStartWaiter) {
    clearTimeout(activeStartWaiter.timer)
    activeStartWaiter = null
  }
}

/** Returns true when the event belonged to the active start operation. */
function settleStartWaiter(
  kind: 'resolve' | 'reject',
  payload: StartResultPayload | Error,
  requestId?: string,
): boolean {
  const waiter = activeStartWaiter
  if (!waiter) {
    return false
  }
  if (requestId !== waiter.requestId) {
    return false
  }
  activeStartWaiter = null
  clearTimeout(waiter.timer)
  if (kind === 'resolve') {
    waiter.resolve(payload as StartResultPayload)
  } else {
    waiter.reject(payload as Error)
  }
  return true
}

async function requestVpnPermission() {
  console.log('prepare vpn')
  const prepare_ret = await prepare_vpn()
  console.log('prepare vpn', JSON.stringify((prepare_ret)))
  if (prepare_ret?.errorMsg?.length) {
    throw new Error(prepare_ret.errorMsg)
  }

  const granted = prepare_ret?.granted ?? true
  if (!granted) {
    console.info('vpn permission request was denied or dismissed')
  }

  return granted
}

function resetVpnConfigStatus() {
  curVpnStatus.ipv4Addr = undefined
  curVpnStatus.ipv4Cidr = undefined
  curVpnStatus.ipv6Addr = undefined
  curVpnStatus.routes = []
  curVpnStatus.dns = undefined
  curVpnStatus.applicationMode = null
  curVpnStatus.appliedPackages = undefined
  curVpnStatus.appliedUids = undefined
  curVpnStatus.skippedPackages = []
  curVpnStatus.attachedInstanceId = undefined
}

function syncVpnStatusFromNative(status: Awaited<ReturnType<typeof get_vpn_status>>) {
  curVpnStatus.running = status?.running ?? false
  if (!curVpnStatus.running) {
    resetVpnConfigStatus()
    return
  }

  const ipv4WithCidr = status?.ipv4Addr
  if (ipv4WithCidr?.length) {
    const [ipv4Addr, cidr] = ipv4WithCidr.split('/')
    curVpnStatus.ipv4Addr = ipv4Addr

    const parsedCidr = Number(cidr)
    curVpnStatus.ipv4Cidr = Number.isInteger(parsedCidr) ? parsedCidr : undefined
  }
  else {
    curVpnStatus.ipv4Addr = undefined
    curVpnStatus.ipv4Cidr = undefined
  }

  curVpnStatus.ipv6Addr = status?.ipv6Addr?.length ? status.ipv6Addr : undefined
  curVpnStatus.routes = [...(status?.routes ?? [])]
  curVpnStatus.dns = status?.dns ?? undefined

  const mode = status?.applicationMode
  curVpnStatus.applicationMode = mode === 'include' || mode === 'all' ? mode : null
  curVpnStatus.appliedPackages = status?.effectivePolicy
    ? [...new Set(status.effectivePolicy.packages ?? [])].sort()
    : (status?.allowedApplications ? [...new Set(status.allowedApplications)].sort() : undefined)
  curVpnStatus.appliedUids = status?.effectivePolicy?.uids
  curVpnStatus.skippedPackages = [...(status?.skippedPackages ?? [])]
}

function generateRequestId(): string {
  return `vpn-${Date.now()}-${Math.random().toString(36).slice(2, 8)}`
}

/**
 * Detaches the Rust side from the current fd (waits for NIC cleanup) before
 * the native interface is closed. A missing instance (network already deleted)
 * is not an error here: the native close alone finishes the stop.
 */
function errorMessage(error: unknown): string {
  return error instanceof Error ? error.message : String(error)
}

async function targetIsCurrent(instanceId: string): Promise<boolean> {
  if (desiredInstanceId !== instanceId) return false
  const ids = await listNetworkInstanceIds()
  return ids.running_inst_ids.some(id => Utils.UuidToStr(id) === instanceId)
}

async function detachRustTun() {
  const instanceId = curVpnStatus.attachedInstanceId
  if (!instanceId) return
  try {
    await setTunFd(-1, instanceId)
  } catch (e) {
    // A deleted instance has already dropped its I/O tasks. Other failures
    // must abort a rebuild: closing a still-owned FD can cause FD reuse bugs.
    const ids = await listNetworkInstanceIds()
    if (ids.running_inst_ids.some(id => Utils.UuidToStr(id) === instanceId)) throw e
  }
  curVpnStatus.attachedInstanceId = undefined
}

async function doStopVpn(force = false) {
  const wasRunning = curVpnStatus.running
  if (!force && !wasRunning) {
    return
  }
  console.log('stop vpn')
  await detachRustTun()
  // stop_vpn resolves after the old interface is closed. Android may keep
  // the service object bound; that does not prevent the next establish.
  const stop_ret = await stop_vpn(generateRequestId())
  console.log('stop vpn', JSON.stringify((stop_ret)))
  curVpnStatus.running = false
  resetVpnConfigStatus()
}

interface StartParams {
  instanceId: string
  ipv4Addr: string
  cidr: number
  ipv6Addr: string | undefined
  routes: string[]
  dns?: string
}

async function doStartVpn(params: StartParams) {
  if (curVpnStatus.running) return
  await ensureAppRoutingPolicyInitialized()
  const policyResp = await get_app_routing_policy()
  const policy = policyResp.policy
  if (!policy) throw new Error('app_policy_missing')
  if (policy.mode === 'include' && !policyResp.effectivePolicy?.uids.length) {
    throw new Error('no_effective_apps')
  }
  // Permission dialogs may stay open much longer than the start timeout.
  // Register the timed native-result waiter only after permission is granted.
  if (!await requestVpnPermission()) throw new Error('vpn_permission_denied')
  if (!await targetIsCurrent(params.instanceId)) throw new Error('vpn_target_changed')
  const request: StartVpnRequest = {
    ipv4Addr: `${params.ipv4Addr}/${params.cidr}`,
    ipv6Addr: params.ipv6Addr,
    routes: params.routes,
    dns: params.dns,
    applicationMode: policy.mode,
    allowedApplications: policy.mode === 'include' ? policy.packages : undefined,
    requestId: generateRequestId(),
    mtu: 1300,
  }
  const resultPromise = expectStartResult(request.requestId!)
  // The native error event can arrive before the start command returns.
  void resultPromise.catch(() => undefined)
  try {
    const response = await start_vpn(request)
    if (response?.errorMsg) throw new Error(response.errorMsg)
    const payload = await resultPromise
    if (!Number.isInteger(payload.fd) || payload.fd! <= 0) throw new Error('invalid_tun_fd')
    curVpnStatus.attachedInstanceId = params.instanceId
    await setTunFd(payload.fd!, params.instanceId)
    if (!await targetIsCurrent(params.instanceId)) throw new Error('vpn_target_changed')
    curVpnStatus.running = true
    curVpnStatus.ipv4Addr = params.ipv4Addr
    curVpnStatus.ipv4Cidr = params.cidr
    curVpnStatus.ipv6Addr = params.ipv6Addr
    curVpnStatus.routes = [...params.routes]
    curVpnStatus.dns = params.dns
    curVpnStatus.applicationMode = policy.mode
    curVpnStatus.appliedPackages = payload.effectivePolicy?.packages
    curVpnStatus.appliedUids = payload.effectivePolicy?.uids
    curVpnStatus.skippedPackages = [...(payload.skippedPackages ?? [])]
    vpnServiceError.value = null
  } catch (error) {
    clearStartWaiter()
    await doStopVpn(true)
    throw error
  }
}

async function onVpnServiceStart(payload: StartResultPayload) {
  if (settleStartWaiter('resolve', payload, payload.requestId)) return
  // A late result of a timed-out/newer request must never attach its old FD.
  if (payload.requestId) return
  // Legacy native startup, confirmed only after Rust has accepted the FD.
  await enqueueVpnOperation(async () => {
    const instanceId = await findRunningTunInstanceId()
    if (!instanceId || !payload.fd) return
    curVpnStatus.attachedInstanceId = instanceId
    try {
      await setTunFd(payload.fd, instanceId)
      syncVpnStatusFromNative(await get_vpn_status())
    } catch (error) {
      await doStopVpn(true)
      vpnServiceError.value = errorMessage(error)
    }
  })
}

async function onVpnServiceStop(payload: { requestId?: string }) {
  // Explicit stop is completed by doStopVpn. It must not reset a newer start.
  if (payload?.requestId) return
  await enqueueVpnOperation(async () => {
    await detachRustTun()
    curVpnStatus.running = false
    resetVpnConfigStatus()
  })
}

async function onVpnServiceError(payload: { requestId?: string; code?: string; message?: string }) {
  const error = new Error(payload?.code || payload?.message || 'vpn_service_error')
  if (settleStartWaiter('reject', error, payload?.requestId)) return
  if (payload?.requestId) return
  vpnServiceError.value = error.message
}

async function registerVpnServiceListener() {
  console.log('register vpn service listener')
  await addPluginListener(
    'vpnservice',
    'vpn_service_start',
    onVpnServiceStart,
  )

  await addPluginListener(
    'vpnservice',
    'vpn_service_stop',
    onVpnServiceStop,
  )

  await addPluginListener(
    'vpnservice',
    'vpn_service_error',
    onVpnServiceError,
  )
}

function getRoutesForVpn(routes: Route[], node_config: NetworkTypes.NetworkConfig): string[] {
  const ret: string[] = []
  for (const r of routes ?? []) {
    for (let cidr of r.proxy_cidrs) {
      if (!cidr.includes('/')) {
        // Family-aware default: a bare IPv6 address is a single host (/128),
        // not a /32 block.
        cidr += cidr.includes(':') ? '/128' : '/32'
      }
      ret.push(cidr)
    }
  }

  node_config.routes.forEach(r => {
    ret.push(r)
  })

  node_config.routes_v6?.forEach(r => {
    ret.push(r)
  })

  // vpn_dns takes over resolution; Magic DNS must not also grab the
  // 100.100.100.101/32 route in that case.
  const useVpnDns = !!node_config.vpn_dns?.length
  if (node_config.enable_magic_dns && !useVpnDns) {
    ret.push('100.100.100.101/32')
  }

  // sort and dedup
  return Array.from(new Set(ret)).sort()
}

// ---------------------------------------------------------------------------
// The single reconcile entry: decides stop/start/rebuild from the latest
// network state and the saved policy.
// ---------------------------------------------------------------------------

async function reconcileOnce(): Promise<void> {
  const instanceId = desiredInstanceId

  if (dhcpPollingTimer) {
    clearTimeout(dhcpPollingTimer)
    dhcpPollingTimer = null
  }

  if (!instanceId) {
    if (curVpnStatus.running) {
      await doStopVpn()
    }
    return
  }

  const config = await getConfig(instanceId)
  if (config.no_tun) {
    if (curVpnStatus.running) await doStopVpn()
    console.log('vpn service skipped because no_tun is enabled', instanceId)
    return
  }

  const curNetworkInfo = (await collectNetworkInfo(instanceId)).info.map[instanceId]
  if (!curNetworkInfo || curNetworkInfo?.error_msg?.length) {
    console.warn('vpn service skipped because network info is unavailable', instanceId, curNetworkInfo?.error_msg)
    if (curVpnStatus.running) {
      await doStopVpn()
    }
    return
  }

  const virtual_ip = Utils.ipv4ToString(curNetworkInfo?.my_node_info?.virtual_ipv4.address)

  if (config.dhcp && (!virtual_ip || !virtual_ip.length)) {
    console.log('DHCP enabled but no IP yet, will retry in', DHCP_POLLING_INTERVAL, 'ms')
    dhcpPollingTimer = setTimeout(() => {
      dhcpPollingTimer = null
      scheduleReconcile('dhcp_retry')
    }, DHCP_POLLING_INTERVAL)
    return
  }

  if (!virtual_ip || !virtual_ip.length) {
    if (curVpnStatus.running) {
      await doStopVpn()
    }
    return
  }

  let network_length = curNetworkInfo?.my_node_info?.virtual_ipv4.network_length
  if (!network_length) {
    network_length = 24
  }

  const routes = getRoutesForVpn(curNetworkInfo?.routes, config)

  // IPv6 has no runtime source (my_node_info carries IPv4 only), so the TUN
  // address comes straight from the configured static CIDR.
  const ipv6Addr = config.virtual_ipv6?.length ? config.virtual_ipv6 : undefined

  const useVpnDns = !!config.vpn_dns?.length
  const dns = useVpnDns
    ? config.vpn_dns
    : (config.enable_magic_dns ? '100.100.100.101' : undefined)

  // Desired app routing: the latest saved policy as resolved natively.
  await ensureAppRoutingPolicyInitialized()
  const policyResp = await get_app_routing_policy()
  const desiredMode = policyResp.policy?.mode === 'include' ? 'include' : 'all'

  const netChanged = netParamsChanged(
    {
      ipv4Addr: curVpnStatus.ipv4Addr ?? undefined,
      ipv4Cidr: curVpnStatus.ipv4Cidr ?? undefined,
      ipv6Addr: curVpnStatus.ipv6Addr ?? undefined,
      routes: curVpnStatus.routes,
      dns: curVpnStatus.dns ?? undefined,
    },
    { ipv4Addr: virtual_ip, ipv4Cidr: network_length, ipv6Addr, routes, dns },
  )

  const appliedRouting: AppliedRoutingState = {
    mode: curVpnStatus.applicationMode,
    packages: curVpnStatus.appliedPackages,
    uids: curVpnStatus.appliedUids,
  }
  const desiredRouting: AppliedRoutingState = {
    mode: desiredMode,
    packages: policyResp.effectivePolicy?.packages,
    uids: policyResp.effectivePolicy?.uids,
  }

  const needsRebuild = shouldRebuildVpn({
    running: curVpnStatus.running,
    netChanged: netChanged || curVpnStatus.attachedInstanceId !== instanceId,
    appliedSignature: routingSignature(appliedRouting),
    desiredSignature: routingSignature(desiredRouting),
  })

  if (!curVpnStatus.running || needsRebuild) {
    console.info('vpn service (re)building', JSON.stringify({
      running: curVpnStatus.running,
      netChanged,
      appliedMode: curVpnStatus.applicationMode,
      desiredMode,
    }))
    if (desiredMode === 'include' && !policyResp.effectivePolicy?.uids.length) {
      if (curVpnStatus.running) await doStopVpn()
      throw new Error('no_effective_apps')
    }
    if (!await targetIsCurrent(instanceId)) return
    if (curVpnStatus.running) await doStopVpn()
    if (!await targetIsCurrent(instanceId)) return
    await doStartVpn({ instanceId, ipv4Addr: virtual_ip, cidr: network_length, ipv6Addr, routes, dns })
  }
}

// ---------------------------------------------------------------------------
// Policy apply flow (page-facing). Validation happens first and never touches
// the running VPN; save + rebuild run inside the serial queue with one
// restore attempt on failure.
// ---------------------------------------------------------------------------

export type ApplyPolicyOutcome =
  | { status: 'applied'; skippedPackages: SkippedPackage[] }
  | { status: 'saved' }
  | { status: 'restored' }
  | { status: 'stopped' }

export async function applyAppRoutingPolicy(nextPolicy: AppRoutingPolicy): Promise<ApplyPolicyOutcome> {
  return enqueueVpnOperation(async () => {
    const pre = await set_app_routing_policy(nextPolicy, true)
    const previous = (await get_app_routing_policy()).policy
    const previousInstanceId = curVpnStatus.attachedInstanceId ?? desiredInstanceId
    const wasRunning = curVpnStatus.running
    await set_app_routing_policy(pre.policy, false)
    if (!wasRunning) return { status: 'saved' }
    try {
      // An unchanged effective UID set needs no interruption; reconcile also
      // picks up any real address/route changes that arrived while editing.
      await reconcileOnce()
      if (!previousInstanceId || !await targetIsCurrent(previousInstanceId)) return { status: 'saved' }
      if (!curVpnStatus.running) throw new Error('vpn_not_running_after_apply')
      return { status: 'applied', skippedPackages: [...curVpnStatus.skippedPackages] }
    } catch (error) {
      // A user stop/delete wins over a policy apply or its recovery attempt.
      if (!previousInstanceId || !await targetIsCurrent(previousInstanceId)) {
        await doStopVpn(true)
        return { status: 'saved' }
      }
      if (!previous) throw error
      // Never report restored if the persistent rollback itself failed.
      await set_app_routing_policy(previous, false)
      try {
        await doStopVpn(true)
        await reconcileOnce()
        if (!await targetIsCurrent(previousInstanceId)) return { status: 'saved' }
        if (!curVpnStatus.running) throw new Error('vpn_not_running_after_restore')
        return { status: 'restored' }
      } catch (restoreError) {
        vpnServiceError.value = errorMessage(restoreError)
        return { status: 'stopped' }
      }
    }
  })
}

export async function isVpnRunning(): Promise<boolean> {
  // A page status query cannot mutate the coordinator's attach/stop phase.
  return !!(await get_vpn_status())?.running
}

async function isNoTunEnabled(instanceId: string | undefined) {
  if (!instanceId) {
    return false
  }
  return (await getConfig(instanceId)).no_tun ?? false
}

async function findRunningTunInstanceId() {
  const instanceIds = await listNetworkInstanceIds()
  const runningIds = instanceIds.running_inst_ids.map(Utils.UuidToStr)
  console.log('vpn service sync running instances', JSON.stringify(runningIds))

  for (const instanceId of runningIds) {
    if (await isNoTunEnabled(instanceId)) {
      continue
    }

    return instanceId
  }

  return undefined
}

export async function initMobileVpnService() {
  await registerVpnServiceListener()
  syncVpnStatusFromNative(await get_vpn_status())
  if (curVpnStatus.running) curVpnStatus.attachedInstanceId = await findRunningTunInstanceId()
  try {
    await ensureAppRoutingPolicyInitialized()
  } catch (e) {
    console.error('app routing policy initialization failed', e)
  }
}

export async function prepareVpnService(instanceId: string) {
  if (await isNoTunEnabled(instanceId)) {
    return
  }
  await requestVpnPermission()
}

export async function syncMobileVpnService() {
  const instanceId = await findRunningTunInstanceId()
  desiredInstanceId = instanceId
  if (instanceId) {
    console.log('vpn service sync selected instance', instanceId)
    scheduleReconcile('sync')
    return
  }

  if (dhcpPollingTimer) {
    clearTimeout(dhcpPollingTimer)
    dhcpPollingTimer = null
  }

  await enqueueVpnOperation(() => doStopVpn(true))
}

export async function onNetworkInstanceChange(instanceId: string) {
  console.log('vpn service network instance change id', instanceId)
  desiredInstanceId = await findRunningTunInstanceId()
  scheduleReconcile('instance_change')
}
