import { invoke } from '@tauri-apps/api/core'

export async function ping(value: string): Promise<string | null> {
  return await invoke<{ value?: string }>('plugin:vpnservice|ping', {
    payload: {
      value,
    },
  }).then((r) => (r.value ? r.value : null));
}

export interface InvokeResponse {
  errorMsg?: string;
  granted?: boolean;
}

export type AppRoutingMode = 'include' | 'all'

export interface AppRoutingPolicy {
  schemaVersion: number
  mode: AppRoutingMode
  /** Saved selection for include mode; deduped, sorted, self-uid excluded. */
  packages: string[]
}

export interface InstalledApp {
  packageName: string
  label: string
  uid: number
  isSystem: boolean
  hasLauncher: boolean
  enabled: boolean
  hasInternetPermission: boolean
  /** Other packages sharing this app's uid; selecting one affects the group. */
  sharedUidPackages: string[]
  selectable: boolean
  unavailableReason?: 'disabled' | 'vpn_self_uid'
  /** packageName + lastUpdateTime; used as the in-memory icon cache key. */
  iconKey: string
}

export interface AppCatalogResponse {
  selfPackageName: string
  selfUid: number
  apps: InstalledApp[]
}

export interface AppIconResponse {
  packageName: string
  iconKey: string
  /** PNG data URL of the system-chosen icon; absent when unavailable. */
  dataUrl?: string
}

export type SkippedPackageReason = 'not_installed' | 'disabled' | 'vpn_self_uid'

export interface SkippedPackage {
  packageName: string
  reason: SkippedPackageReason
}

export interface EffectiveAppRoutingPolicy {
  mode: AppRoutingMode
  /** Installed + enabled packages the builder will capture, uid groups expanded. */
  packages: string[]
  /** Distinct uids covered; runtime only, never persisted. */
  uids: number[]
}

export interface AppRoutingPolicyResult {
  policy: AppRoutingPolicy
  effectivePolicy: EffectiveAppRoutingPolicy
  skippedPackages: SkippedPackage[]
}

export interface GetAppRoutingPolicyResponse {
  /** null when no policy has ever been saved on this device. */
  policy: AppRoutingPolicy | null
  effectivePolicy?: EffectiveAppRoutingPolicy
  skippedPackages: SkippedPackage[]
}

export interface StartVpnRequest {
  ipv4Addr?: string;
  ipv4Addrs?: string[];
  routes?: string[];
  dns?: string;
  applicationMode?: AppRoutingMode;
  allowedApplications?: string[];
  requestId?: string;
  disallowedApplications?: string[];
  mtu?: number;
}

export interface VpnStatusResponse {
  running: boolean;
  applicationMode?: AppRoutingMode | null;
  effectivePolicy?: EffectiveAppRoutingPolicy;
  skippedPackages?: SkippedPackage[];
  lockdownEnabled?: boolean;
  ipv4Addr?: string;
  ipv4Addrs?: string[];
  routes?: string[];
  dns?: string;
}

export type VpnTileAction = 'start' | 'stop';

export interface VpnTileActionResponse {
  action?: VpnTileAction;
}

export async function prepare_vpn(): Promise<InvokeResponse | null> {
  return await invoke<InvokeResponse>('plugin:vpnservice|prepare_vpn', {})
}

export async function start_vpn(request: StartVpnRequest): Promise<InvokeResponse | null> {
  return await invoke<InvokeResponse>('plugin:vpnservice|start_vpn', {
    ...request,
  })
}

export async function stop_vpn(): Promise<InvokeResponse | null> {
  return await invoke<InvokeResponse>('plugin:vpnservice|stop_vpn', {})
}

export async function get_vpn_status(): Promise<VpnStatusResponse | null> {
  return await invoke<VpnStatusResponse>('plugin:vpnservice|get_vpn_status', {})
}

export async function consume_vpn_tile_action(): Promise<VpnTileActionResponse> {
  return await invoke<VpnTileActionResponse>('plugin:vpnservice|consume_vpn_tile_action', {})
}

/** Enables native system-back interception only while the full-screen page is open. */
export async function set_app_routing_page_open(open: boolean): Promise<void> {
  await invoke('plugin:vpnservice|set_app_routing_page_open', { open })
}

export async function get_installed_apps(): Promise<AppCatalogResponse> {
  return await invoke<AppCatalogResponse>('plugin:vpnservice|get_installed_apps', {})
}

export async function get_app_icon(packageName: string): Promise<AppIconResponse> {
  return await invoke<AppIconResponse>('plugin:vpnservice|get_app_icon', {
    packageName,
  })
}

export async function get_app_routing_policy(): Promise<GetAppRoutingPolicyResponse> {
  return await invoke<GetAppRoutingPolicyResponse>('plugin:vpnservice|get_app_routing_policy', {})
}

/**
 * Validates (validateOnly) or persists the policy. Never restarts the VPN by
 * itself; the GUI drives rebuilds through the serial reconcile entry.
 * validateOnly=true rejects an include policy with zero effective apps.
 */
export async function set_app_routing_policy(
  policy: AppRoutingPolicy,
  validateOnly?: boolean,
): Promise<AppRoutingPolicyResult> {
  return await invoke<AppRoutingPolicyResult>('plugin:vpnservice|set_app_routing_policy', {
    policy,
    validateOnly,
  })
}
