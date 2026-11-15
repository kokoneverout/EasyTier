<script setup lang="ts">
import { computed, nextTick, onMounted, onUnmounted, reactive, ref, watch } from 'vue'
import { useI18n } from 'vue-i18n'
import type { AppRoutingPolicy, InstalledApp } from 'tauri-plugin-vpnservice-api'
import { addPluginListener } from '@tauri-apps/api/core'
import { get_vpn_status, get_app_routing_policy, set_app_routing_page_open } from 'tauri-plugin-vpnservice-api'
import { cachedIcon, dropIconCache, fetchAppCatalog, loadIcon, ensureAppRoutingPolicyInitialized } from '~/composables/app_routing'
import { applyAppRoutingPolicy, isVpnRunning } from '~/composables/mobile_vpn'

/**
 * Full-screen per-app routing page. The draft (mode + selected packages)
 * only becomes effective after "apply"; the VPN rebuild itself is owned by
 * mobile_vpn's serial reconcile entry.
 */

const emit = defineEmits<{ (e: 'close'): void }>()

const { t, locale } = useI18n()

// ---- catalog & policy state ------------------------------------------------

const loading = ref(true)
const loadError = ref(false)
const apps = ref<InstalledApp[]>([])
const selfUid = ref(-1)

const savedPolicy = ref<AppRoutingPolicy | null>(null)
const legacyAllNote = ref(false)
const lockdownEnabled = ref(false)
const vpnRunning = ref(false)

// draft state
const draftMode = ref<'include' | 'all'>('include')
const selected = reactive(new Set<string>())
const query = ref('')
const showSystem = ref(false)

// apply state
const applying = ref(false)
const applyStatus = ref<'' | 'unapplied' | 'applied' | 'next_start' | 'restored' | 'stopped'>('')
const applySkippedCount = ref(0)
const applyError = ref('')
const discardOpen = ref(false)
const rootEl = ref<HTMLElement | null>(null)

const appsByPackage = computed(() => {
  const map = new Map<string, InstalledApp>()
  for (const app of apps.value) {
    map.set(app.packageName, app)
  }
  return map
})

// Uninstalled (or otherwise missing) packages kept in the saved selection.
const uninstalledSelected = computed(() =>
  [...selected].filter(pkg => !appsByPackage.value.has(pkg)).sort(),
)

const sharedUidNoticePackages = computed(() => {
  // Names of apps that share a uid with at least one *selected* app.
  const names: string[] = []
  for (const pkg of selected) {
    const app = appsByPackage.value.get(pkg)
    if (!app || app.sharedUidPackages.length === 0) {
      continue
    }
    for (const other of app.sharedUidPackages) {
      const otherApp = appsByPackage.value.get(other)
      if (otherApp && otherApp.selectable) {
        names.push(otherApp.label)
      }
    }
  }
  return [...new Set(names)].sort((a, b) => collator.value.compare(a, b)).slice(0, 6)
})

// ---- filtering & sorting ---------------------------------------------------

const collator = computed(() => new Intl.Collator(locale.value || undefined, { numeric: true, sensitivity: 'base' }))

function matchesSearch(app: InstalledApp, q: string): boolean {
  return app.label.toLocaleLowerCase().includes(q) || app.packageName.toLocaleLowerCase().includes(q)
}

function passesSystemFilter(app: InstalledApp): boolean {
  // Selected apps never hide behind the system-service filter.
  if (selected.has(app.packageName)) {
    return true
  }
  if (!app.isSystem || app.hasLauncher) {
    return true
  }
  return showSystem.value
}

interface DisplayRow {
  key: string
  packageName: string
  label: string
  isUninstalled: boolean
  app?: InstalledApp
}

const visibleRows = computed<DisplayRow[]>(() => {
  if (draftMode.value === 'all') {
    return []
  }
  const q = query.value.trim().toLocaleLowerCase()

  const installedRows: DisplayRow[] = []
  for (const app of apps.value) {
    if (!passesSystemFilter(app)) {
      continue
    }
    if (q && !matchesSearch(app, q)) {
      // Search filters everything, selected rows included.
      continue
    }
    installedRows.push({ key: app.packageName, packageName: app.packageName, label: app.label, isUninstalled: false, app })
  }

  const uninstalledRows: DisplayRow[] = uninstalledSelected.value
    .filter(pkg => !q || pkg.toLocaleLowerCase().includes(q))
    .map(pkg => ({ key: pkg, packageName: pkg, label: pkg, isUninstalled: true }))

  const c = collator.value
  const byLabel = (a: DisplayRow, b: DisplayRow) => c.compare(a.label, b.label) || c.compare(a.packageName, b.packageName)

  const selectedInstalled = installedRows.filter(r => selected.has(r.packageName)).sort(byLabel)
  const unselectedInstalled = installedRows.filter(r => !selected.has(r.packageName)).sort(byLabel)
  // Invalid selected entries sort to the end of the selected group.
  const selectedUninstalled = uninstalledRows.sort(byLabel)

  return [...selectedInstalled, ...selectedUninstalled, ...unselectedInstalled]
})

const selectedCount = computed(() => selected.size)
const foundCount = computed(() => visibleRows.value.length)
const hasSelection = computed(() => selected.size > 0)
const searchable = computed(() => draftMode.value === 'include')

// ---- selection -------------------------------------------------------------

async function toggleRow(row: DisplayRow) {
  const anchor = Array.from(rootEl.value?.querySelectorAll<HTMLElement>('.ar-card') ?? [])
    .find(el => el.dataset.package !== row.packageName && el.getBoundingClientRect().top >= (rootEl.value?.querySelector('.ar-list')?.getBoundingClientRect().top ?? 0))
  const anchorTop = anchor?.getBoundingClientRect().top
  const scroller = rootEl.value?.querySelector<HTMLElement>('.ar-list')
  if (applying.value) {
    return
  }
  if (row.isUninstalled) {
    // Saved-but-missing entries can only be deselected.
    selected.delete(row.packageName)
    markDirty()
    return
  }
  const app = row.app!
  if (!app.selectable && !selected.has(app.packageName)) {
    return
  }
  if (selected.has(app.packageName)) {
    removeFromGroup(app)
  } else {
    selected.add(app.packageName)
    for (const other of app.sharedUidPackages) {
      if (appsByPackage.value.get(other)?.selectable) {
        selected.add(other)
      }
    }
  }
  markDirty()
  await nextTick()
  if (scroller && anchor?.isConnected && anchorTop !== undefined) {
    scroller.scrollTop += anchor.getBoundingClientRect().top - anchorTop
  }
}

function removeFromGroup(app: InstalledApp) {
  selected.delete(app.packageName)
  for (const other of app.sharedUidPackages) {
    selected.delete(other)
  }
}

function clearSelection() {
  if (applying.value || selected.size === 0) {
    return
  }
  selected.clear()
  markDirty()
}

function markDirty() {
  if (applying.value) {
    return
  }
  applyStatus.value = 'unapplied'
  applyError.value = ''
}

// ---- icons (metadata first, lazy rasterization, <=4 concurrent) ------------

const rowEls = new Map<string, HTMLElement>()
const iconUrls = reactive(new Map<string, string>())
let iconObserver: IntersectionObserver | null = null
let iconConcurrency = 0
let pageDisposed = false
const queuedIconKeys = new Set<string>()
const iconQueue: Array<() => void> = []

function setRowEl(key: string, el: unknown) {
  const element = el as HTMLElement | null
  if (element) {
    rowEls.set(key, element)
  } else {
    rowEls.delete(key)
  }
}

function pumpIconQueue() {
  if (pageDisposed) return
  while (iconConcurrency < 4 && iconQueue.length > 0) {
    const job = iconQueue.shift()!
    iconConcurrency++
    job()
  }
}

function queueIcon(key: string, packageName: string) {
  if (pageDisposed || queuedIconKeys.has(key)) return
  if (cachedIcon(key) !== undefined) {
    const cached = cachedIcon(key)
    if (cached) {
      iconUrls.set(key, cached)
    }
    return
  }
  queuedIconKeys.add(key)
  iconQueue.push(() => {
    loadIcon(key, packageName)
      .then((dataUrl) => {
        if (dataUrl && !pageDisposed) {
          iconUrls.set(key, dataUrl)
        }
      })
      .finally(() => {
        iconConcurrency--
        queuedIconKeys.delete(key)
        pumpIconQueue()
      })
  })
  pumpIconQueue()
}

function syncIconObserver() {
  if (!iconObserver) {
    return
  }
  iconObserver.disconnect()
  if (draftMode.value !== 'include') {
    return
  }
  for (const row of visibleRows.value) {
    if (row.isUninstalled) {
      continue
    }
    const el = rowEls.get(row.app!.iconKey)
    if (el) {
      iconObserver.observe(el)
    }
  }
}

watch(visibleRows, () => {
  void nextTick(syncIconObserver)
})

// ---- loading ---------------------------------------------------------------

async function loadCatalog() {
  loading.value = true
  loadError.value = false
  try {
    await ensureAppRoutingPolicyInitialized()
    const [catalog, vpnStatus] = await Promise.all([
      fetchAppCatalog(),
      get_vpn_status(),
    ])
    apps.value = catalog.apps
    selfUid.value = catalog.selfUid
    lockdownEnabled.value = !!vpnStatus?.lockdownEnabled
    vpnRunning.value = !!vpnStatus?.running
  } catch (e) {
    console.error('app routing catalog load failed', e)
    loadError.value = true
  } finally {
    loading.value = false
  }
}

async function loadPolicy(resetDraft = true) {
  const resp = await get_app_routing_policy()
  if (!resp.policy) throw new Error('app_policy_missing')
  const packages = new Set(resp.policy.packages)
  for (const pkg of resp.policy.packages) {
    const app = appsByPackage.value.get(pkg)
    if (app?.selectable) {
      for (const member of app.sharedUidPackages) packages.add(member)
    }
  }
  savedPolicy.value = { ...resp.policy, packages: [...packages].sort() }
  if (resetDraft) {
    draftMode.value = resp.policy.mode
    selected.clear()
    for (const pkg of packages) selected.add(pkg)
  }
}

async function reload() {
  await loadCatalog()
  if (loadError.value) return
  try {
    await loadPolicy()
    void nextTick(syncIconObserver)
  } catch (error) {
    console.error('app routing policy load failed', error)
    loadError.value = true
  }
}

function onVisibilityChange() {
  if (document.visibilityState === 'visible' && !applying.value) {
    // Refresh the catalog on re-entry; drafts without apply are not part of
    // the policy and stay as they are.
    void loadCatalog()
  }
}

// ---- apply -----------------------------------------------------------------

async function onApply() {
  if (applying.value || !canApply.value) return
  applying.value = true
  applyError.value = ''
  try {
    const outcome = await applyAppRoutingPolicy({ schemaVersion: 1, mode: draftMode.value, packages: [...selected] })
    legacyAllNote.value = false
    const succeeded = outcome.status === 'applied' || outcome.status === 'saved'
    await loadPolicy(succeeded)
    applyStatus.value = outcome.status === 'saved' ? 'next_start' : outcome.status
    applySkippedCount.value = outcome.status === 'applied' ? outcome.skippedPackages.length : 0
    vpnRunning.value = await isVpnRunning()
  } catch (error) {
    applyError.value = String(error instanceof Error ? error.message : error)
    applyStatus.value = 'unapplied'
  } finally {
    applying.value = false
  }
}

// ---- close -----------------------------------------------------------------

const dirty = computed(() => {
  if (!savedPolicy.value) {
    return draftMode.value !== 'include' || selected.size > 0
  }
  if (draftMode.value !== savedPolicy.value.mode) {
    return true
  }
  const saved = new Set(savedPolicy.value.packages)
  if (saved.size !== selected.size) {
    return true
  }
  for (const pkg of selected) {
    if (!saved.has(pkg)) {
      return true
    }
  }
  return false
})

const canApply = computed(() => !loading.value && !loadError.value && dirty.value
  && (draftMode.value === 'all' || [...selected].some(pkg => appsByPackage.value.get(pkg)?.selectable)))
let historyBound = false
let backListener: Awaited<ReturnType<typeof addPluginListener>> | undefined
const previousFocus = document.activeElement as HTMLElement | null

function closePage() {
  window.removeEventListener('popstate', onPopState)
  if (historyBound) {
    historyBound = false
    history.back()
  }
  emit('close')
}

function requestClose() {
  if (applying.value) return
  if (dirty.value) discardOpen.value = true
  else closePage()
}

function rebindHistory() {
  history.pushState({ ...history.state, appRouting: true }, '')
  historyBound = true
}

function onPopState() {
  historyBound = false
  // Hold this page while applying or showing discard confirmation.
  rebindHistory()
  requestClose()
}

function onKeydown(event: KeyboardEvent) {
  if (event.key === 'Escape') { event.preventDefault(); requestClose(); return }
  if (event.key !== 'Tab') return
  const scope = discardOpen.value ? rootEl.value?.querySelector('.ar-discard') : rootEl.value
  const buttons = Array.from(scope?.querySelectorAll<HTMLElement>('button:not(:disabled), input:not(:disabled)') ?? [])
  const first = buttons[0], last = buttons.at(-1)
  if (event.shiftKey && document.activeElement === first) { event.preventDefault(); last?.focus() }
  else if (!event.shiftKey && document.activeElement === last) { event.preventDefault(); first?.focus() }
}
watch(discardOpen, async (open) => {
  if (open) { await nextTick(); rootEl.value?.querySelector<HTMLElement>('.ar-discard button')?.focus() }
})

onMounted(async () => {
  iconObserver = new IntersectionObserver((entries) => {
    for (const entry of entries) {
      if (entry.isIntersecting) {
        const key = (entry.target as HTMLElement).dataset.rowKey
        if (key) {
          iconObserver?.unobserve(entry.target)
          queueIcon(key, key.split('@')[0])
        }
      }
    }
  }, { rootMargin: '240px 0px' })

  backListener = await addPluginListener('vpnservice', 'app_routing_back', requestClose)
  await set_app_routing_page_open(true)
  rebindHistory()
  window.addEventListener('popstate', onPopState)
  document.addEventListener('visibilitychange', onVisibilityChange)

  await reload()
  await nextTick()
  rootEl.value?.querySelector<HTMLElement>('button')?.focus()
})

onUnmounted(() => {
  pageDisposed = true
  iconObserver?.disconnect()
  iconObserver = null
  iconQueue.length = 0
  queuedIconKeys.clear()
  dropIconCache()
  void backListener?.unregister()
  void set_app_routing_page_open(false)
  previousFocus?.focus()
  window.removeEventListener('popstate', onPopState)
  document.removeEventListener('visibilitychange', onVisibilityChange)
})
</script>

<template>
  <div ref="rootEl" class="ar-root" @keydown="onKeydown" role="dialog" aria-modal="true" :aria-label="t('app_routing.title')">
    <header class="ar-topbar">
      <button type="button" class="ar-topbar-btn" :aria-label="t('app_routing.back')"
        :disabled="applying" @click="requestClose">
        <svg viewBox="0 0 24 24" width="24" height="24" aria-hidden="true">
          <path d="M20 11H7.83l5.59-5.59L12 4l-8 8 8 8 1.41-1.41L7.83 13H20v-2z" fill="currentColor" />
        </svg>
      </button>
      <h1 class="ar-title">{{ t('app_routing.title') }}</h1>
      <button type="button" class="ar-topbar-btn ar-apply-btn" :disabled="applying || !canApply"
        @click="onApply">
        {{ applying ? t('app_routing.applying') : t('app_routing.apply') }}
      </button>
    </header>

    <p v-if="applyError" class="ar-hint ar-error" role="alert">{{ applyError.includes('no_effective_apps') ? t('app_routing.apply_failed_empty') : t('app_routing.apply_failed') }}</p>
    <div class="ar-scroll">
      <div class="ar-content">
        <!-- mode + count + hint -->
        <section class="ar-mode-block">
          <div class="ar-mode-row">
            <div class="ar-mode-toggle" role="radiogroup" :aria-label="t('app_routing.title')">
              <button type="button" role="radio" :aria-checked="draftMode === 'include'"
                class="ar-mode-option" :class="{ 'ar-mode-option-on': draftMode === 'include' }"
                :disabled="applying" @click="draftMode = 'include'; markDirty()">
                {{ t('app_routing.mode_include') }}
              </button>
              <button type="button" role="radio" :aria-checked="draftMode === 'all'"
                class="ar-mode-option" :class="{ 'ar-mode-option-on': draftMode === 'all' }"
                :disabled="applying" @click="draftMode = 'all'; markDirty()">
                {{ t('app_routing.mode_all') }}
              </button>
            </div>
            <span v-if="draftMode === 'include'" class="ar-count" aria-live="polite">
              {{ t('app_routing.selected_count', { count: selectedCount }) }}
            </span>
          </div>
          <p class="ar-hint">
            {{ draftMode === 'include' ? t('app_routing.include_hint') : t('app_routing.all_hint') }}
          </p>
          <p v-if="legacyAllNote" class="ar-hint">{{ t('app_routing.legacy_all_note') }}</p>
          <p v-if="lockdownEnabled" class="ar-hint" role="note">{{ t('app_routing.lockdown_note') }}</p>
          <p class="ar-status" :class="{ 'ar-status-dirty': applyStatus === 'unapplied' }">
            <template v-if="applying">{{ t('app_routing.applying') }}</template>
            <template v-else-if="applyStatus === 'unapplied'">{{ t('app_routing.status_unapplied') }}</template>
            <template v-else-if="applyStatus === 'applied' && applySkippedCount > 0">
              {{ t('app_routing.status_applied_with_skipped', { count: applySkippedCount }) }}
            </template>
            <template v-else-if="applyStatus === 'applied'">{{ t('app_routing.status_applied') }}</template>
            <template v-else-if="applyStatus === 'next_start'">{{ t('app_routing.status_next_start') }}</template>
            <template v-else-if="applyStatus === 'restored'">{{ t('app_routing.status_restored') }}</template>
            <template v-else-if="applyStatus === 'stopped'">{{ t('app_routing.status_stopped') }}</template>
          </p>
          <p v-if="uninstalledSelected.length > 0 && draftMode === 'include'" class="ar-hint">
            {{ t('app_routing.uninstalled_apps', { count: uninstalledSelected.length }) }}
          </p>
          <p v-if="sharedUidNoticePackages.length > 0 && draftMode === 'include'" class="ar-hint">
            {{ t('app_routing.shared_uid_note') }}：{{ sharedUidNoticePackages.join('、') }}
          </p>
        </section>

        <!-- search + filters (include mode only) -->
        <section v-if="searchable" class="ar-search-block">
          <div class="ar-search-row">
            <input v-model="query" type="search" class="ar-search" :placeholder="t('app_routing.search_placeholder')"
              :disabled="applying" :aria-label="t('app_routing.search_placeholder')">
            <button v-if="query" type="button" class="ar-text-btn" @click="query = ''">
              {{ t('app_routing.clear_search') }}
            </button>
          </div>
          <div class="ar-search-row ar-filter-row">
            <label class="ar-system-toggle">
              <input v-model="showSystem" type="checkbox" class="ar-checkbox" :disabled="applying">
              <span>{{ t('app_routing.show_system') }}</span>
            </label>
            <button type="button" class="ar-text-btn" :disabled="applying || !hasSelection" @click="clearSelection">
              {{ t('app_routing.clear_selection') }}
            </button>
            <span v-if="query" class="ar-count">{{ t('app_routing.found_count', { count: foundCount }) }}</span>
          </div>
        </section>

        <!-- list -->
        <section class="ar-list">
          <div v-if="loading" class="ar-empty">{{ t('app_routing.loading') }}</div>
          <div v-else-if="loadError" class="ar-empty" role="alert">
            <p>{{ t('app_routing.load_failed') }}</p>
            <button type="button" class="ar-text-btn" @click="reload">{{ t('app_routing.retry') }}</button>
          </div>
          <template v-else-if="searchable">
            <div v-if="visibleRows.length === 0" class="ar-empty">
              <template v-if="query">{{ t('app_routing.no_match') }}</template>
              <template v-else-if="selectedCount === 0">{{ t('app_routing.empty_selection_hint') }}</template>
            </div>
            <button v-for="row in visibleRows" :key="row.key" type="button" class="ar-card"
              :class="{ 'ar-card-selected': selected.has(row.packageName) }"
              :aria-pressed="selected.has(row.packageName)"
              :data-package="row.packageName"
              :data-row-key="row.isUninstalled ? undefined : row.app!.iconKey"
              :ref="el => setRowEl(row.isUninstalled ? row.key : row.app!.iconKey, el)"
              :disabled="applying || (!!row.app && !row.app.selectable && !selected.has(row.packageName))"
              @click="toggleRow(row)">
              <img v-if="!row.isUninstalled && iconUrls.get(row.app!.iconKey)" class="ar-icon"
                :src="iconUrls.get(row.app!.iconKey)" alt="" width="48" height="48">
              <span v-else class="ar-icon ar-icon-placeholder" aria-hidden="true"></span>
              <span class="ar-texts">
                <span class="ar-name" :class="{ 'ar-name-disabled': !!row.app && !row.app.selectable }">
                  {{ row.isUninstalled ? t('app_routing.uninstalled_label') : row.label }}
                </span>
                <span class="ar-package">{{ row.packageName }}</span>
                <span class="ar-sr-only">
                  {{ selected.has(row.packageName) ? t('app_routing.selected_state_on') : t('app_routing.selected_state_off') }}
                </span>
              </span>
            </button>
          </template>
        </section>
      </div>
    </div>
    <div v-if="discardOpen" class="ar-discard-overlay">
      <section class="ar-discard" role="alertdialog" :aria-label="t('app_routing.discard_confirm')">
        <p>{{ t('app_routing.discard_confirm') }}</p>
        <button class="ar-text-btn" @click="discardOpen = false">{{ t('app_routing.keep_editing') }}</button>
        <button class="ar-text-btn" @click="closePage">{{ t('app_routing.discard') }}</button>
      </section>
    </div>
  </div>
</template>

<style scoped lang="css">
.ar-root {
  color-scheme: light;
  --ar-page: #EFEEEA;
  --ar-card: #FFFFFF;
  --ar-name: #1A1A1A;
  --ar-package: #6F6F6A;
  --ar-selected: #0B6B58;
  --ar-selected-name: #FFFFFF;
  --ar-selected-package: #CDE6DF;
  --ar-focus: #6F6F6A;
  --ar-disabled-name: #6F6F6A;

  position: fixed;
  inset: 0;
  z-index: 1200;
  display: flex;
  flex-direction: column;
  background: var(--ar-page);
  color: var(--ar-name);
  overflow: hidden;
}

@media (prefers-color-scheme: dark) {
  .ar-root {
    color-scheme: dark;
    --ar-page: #0E0E10;
    --ar-card: #1A1A1D;
    --ar-name: #F2F2F3;
    --ar-package: #8E8E93;
    --ar-selected: #137A64;
    --ar-selected-name: #FFFFFF;
    --ar-selected-package: #DEF3EC;
    --ar-focus: #B8B8BD;
    --ar-disabled-name: #8E8E93;
  }
}

.ar-topbar {
  flex: none;
  min-height: 56px;
  display: flex;
  align-items: center;
  gap: 8px;
  padding: env(safe-area-inset-top) 8px 0;
  background: var(--ar-page);
}

.ar-title {
  flex: 1 1 auto;
  min-width: 0;
  margin: 0;
  font-size: 20px;
  line-height: 28px;
  font-weight: 600;
  overflow-wrap: anywhere;
}

.ar-topbar-btn {
  flex: none;
  min-width: 48px;
  min-height: 48px;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  border: 0;
  border-radius: 12px;
  background: transparent;
  color: var(--ar-name);
  font-size: 16px;
}

.ar-apply-btn {
  background: var(--ar-name);
  color: var(--ar-card);
  font-weight: 600;
  padding: 0 16px;
}

.ar-topbar-btn:disabled {
  opacity: 0.5;
}

.ar-scroll {
  flex: 1 1 auto;
  display: flex;
  min-height: 0;
  overflow: hidden;
}

.ar-content {
  width: 100%;
  max-width: 640px;
  margin: 0 auto;
  padding: 0 16px;
  display: flex;
  flex-direction: column;
  min-height: 0;
}

.ar-mode-block {
  padding-top: 8px;
}

.ar-mode-row {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
  flex-wrap: wrap;
}

.ar-mode-toggle {
  display: inline-flex;
  border-radius: 12px;
  background: var(--ar-card);
  overflow: hidden;
}

.ar-mode-option {
  min-height: 48px;
  padding: 0 16px;
  border: 0;
  background: transparent;
  color: var(--ar-package);
  font-size: 14px;
}

.ar-mode-option-on {
  background: var(--ar-name);
  color: var(--ar-card);
  font-weight: 600;
}

.ar-mode-option:disabled {
  opacity: 0.5;
}

.ar-count {
  font-size: 13px;
  line-height: 20px;
  color: var(--ar-package);
}

.ar-hint {
  margin: 8px 0 0;
  font-size: 13px;
  line-height: 20px;
  color: var(--ar-package);
  overflow-wrap: anywhere;
}

.ar-status {
  margin: 8px 0 0;
  font-size: 13px;
  line-height: 20px;
  color: var(--ar-package);
}

.ar-status-dirty {
  color: var(--ar-name);
  font-weight: 600;
}

.ar-search-block {
  margin-top: 16px;
}

.ar-search-row {
  display: flex;
  align-items: center;
  gap: 8px;
}

.ar-filter-row {
  min-height: 48px;
  flex-wrap: wrap;
}

.ar-search {
  flex: 1 1 auto;
  min-width: 0;
  min-height: 48px;
  padding: 0 16px;
  border: 0;
  border-radius: 12px;
  background: var(--ar-card);
  color: var(--ar-name);
  font-size: 16px;
  outline: none;
}

.ar-search::placeholder {
  color: var(--ar-package);
}

.ar-system-toggle {
  display: inline-flex;
  align-items: center;
  gap: 8px;
  min-height: 48px;
  font-size: 14px;
  color: var(--ar-name);
}

.ar-checkbox {
  width: 18px;
  height: 18px;
  accent-color: var(--ar-name);
}

.ar-text-btn {
  border: 0;
  background: transparent;
  color: var(--ar-package);
  font-size: 14px;
  min-height: 48px;
  padding: 0 8px;
}

.ar-text-btn:disabled {
  opacity: 0.5;
}

.ar-list {
  margin-top: 8px;
  flex: 1;
  min-height: 0;
  overflow-y: auto;
  padding-bottom: calc(env(safe-area-inset-bottom) + 24px);
  overscroll-behavior: contain;
  display: flex;
  flex-direction: column;
  gap: 8px;
}

.ar-empty {
  padding: 32px 0;
  text-align: center;
  color: var(--ar-package);
  font-size: 14px;
  line-height: 22px;
}

.ar-card {
  flex-shrink: 0;
  width: 100%;
  min-height: 80px;
  box-sizing: border-box;
  padding: 16px;
  border: 0;
  border-radius: 12px;
  background: var(--ar-card);
  color: var(--ar-name);
  display: flex;
  align-items: center;
  gap: 12px;
  text-align: left;
  box-shadow: none;
  transition: background-color 120ms, color 120ms;
  overflow-anchor: auto;
}

@media (prefers-reduced-motion: reduce) {
  .ar-card {
    transition: none;
  }
}

.ar-card-selected {
  background: var(--ar-selected);
  color: var(--ar-selected-name);
}

.ar-card:focus-visible {
  outline: 2px solid var(--ar-focus);
  outline-offset: 2px;
}

.ar-card:disabled {
  opacity: 1;
}

.ar-icon {
  flex: none;
  width: 48px;
  height: 48px;
  object-fit: contain;
}

.ar-icon-placeholder {
  display: inline-block;
  border-radius: 12px;
  background: var(--ar-page);
}

.ar-card-selected .ar-icon-placeholder {
  background: var(--ar-selected);
}

.ar-texts {
  flex: 1 1 auto;
  min-width: 0;
  display: flex;
  flex-direction: column;
  gap: 4px;
}

.ar-name {
  font-size: 16px;
  line-height: 22px;
  font-weight: 600;
  overflow-wrap: anywhere;
}

.ar-name-disabled {
  color: var(--ar-disabled-name);
}

.ar-card-selected .ar-name-disabled {
  color: var(--ar-selected-name);
}

.ar-package {
  font-size: 12px;
  line-height: 18px;
  font-weight: 400;
  color: var(--ar-package);
  overflow-wrap: anywhere;
}

.ar-card-selected .ar-package {
  color: var(--ar-selected-package);
}

.ar-sr-only {
  position: absolute;
  width: 1px;
  height: 1px;
  padding: 0;
  margin: -1px;
  overflow: hidden;
  clip: rect(0, 0, 0, 0);
  white-space: nowrap;
  border: 0;
}
.ar-topbar-btn:focus-visible, .ar-text-btn:focus-visible, .ar-mode-option:focus-visible,
.ar-search:focus-visible, .ar-checkbox:focus-visible { outline: 2px solid var(--ar-focus); outline-offset: 2px; }
.ar-error { margin: 0; padding: 8px 16px; }
.ar-discard-overlay { position: absolute; inset: 0; display: grid; place-items: center; background: var(--ar-page); padding: 24px; }
.ar-discard { max-width: 360px; background: var(--ar-card); padding: 24px; border-radius: 12px; font-size: 16px; }
</style>
