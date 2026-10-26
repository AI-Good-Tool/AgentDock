// Shared reactive state and actions for every page.

import { reactive, ref, watch } from 'vue'
import { Message, Modal } from '@arco-design/web-vue'
import { call, desktop as isDesktop, previewMode, type AppSnapshot, type RequestLog, type RouteView, type Target, type AccountRuntime, type ProviderKey } from './api'
import { langSetting, tzSetting, t } from './i18n'

export const PROVIDERS: Array<{ key: ProviderKey; label: string; mark: string }> = [
  { key: 'workbuddy', label: 'WorkBuddy', mark: 'W' },
  { key: 'qoder', label: 'Qoder', mark: 'Q' },
  { key: 'trae', label: 'Trae', mark: 'T' },
]
export const providerLabel = (p?: string) => PROVIDERS.find((x) => x.key === p)?.label ?? (p || '—')
export const regionLabel = (r?: string) => (r === 'cn' ? t('国内版', 'China') : r === 'intl' ? t('国际版', 'International') : '—')
/** Same short form the backend stores in records ("ad-1…9f0e"). */
export function keyHint(value: string) {
  const [head, tail] = value.length >= 24 ? [4, 4] : [2, 2]
  return value.length <= head + tail ? '…' : `${value.slice(0, head)}…${value.slice(-tail)}`
}
/** Which local API key a record came from: the key's current alias or abbreviation; for deleted keys, what the record stored. */
export function keyName(r: { keyId?: string; keyHint?: string; keyAlias?: string }) {
  if (!r.keyId) return ''
  const live = state.app?.apiKeys?.find((k) => k.id === r.keyId)
  if (live) return live.alias || keyHint(live.key)
  return r.keyAlias || r.keyHint || r.keyId
}

export const state = reactive({
  app: null as AppSnapshot | null,
  pool: null as any,
  catalog: null as null | { routes: RouteView[]; upstreams: Target[]; status: any },
  logs: [] as RequestLog[],
  active: [] as RequestLog[],
  pending: new Set<string>(),
  now: Date.now(),
})
export const page = ref('overview')
/** Show full phone numbers on account cards (session only, never persisted). */
export const revealPhones = ref(false)

export function runtimeOf(provider: string, id: string): AccountRuntime | undefined {
  return state.pool?.runtime?.[`${provider}/${id}`]
}
export const isDisabled = (provider: string, id: string) => !!runtimeOf(provider, id)?.disabled
/** Disabling asks first; enabling leaves the proxy off until switched on. */
export function setAccountDisabled(provider: ProviderKey, id: string, disabled: boolean) {
  const run = () =>
    act(
      `disable:${provider}/${id}`,
      () => call('set_account_disabled', { providerKey: provider, id, disabled }),
      disabled
        ? t('账号已禁用，反代已关闭', 'Account disabled; proxy is off')
        : t('账号已启用；需要时请手动开启反代', 'Account enabled; turn the proxy on when needed')
    )
  if (!disabled) return run()
  Modal.confirm({
    title: t('禁用账号', 'Disable account'),
    content: t(
      '禁用后自动关闭反代，且不参与一键刷新积分、刷新限额、全部签到和定时签到。账号数据保留，可随时启用。',
      'The proxy is turned off and the account is left out of batch credit and limit refreshes, check-in for all and scheduled check-ins. Its data is kept; enable it any time.'
    ),
    okText: t('禁用', 'Disable'),
    okButtonProps: { status: 'danger' },
    onOk: run,
  })
}

async function guard<T>(work: () => Promise<T>): Promise<T | undefined> {
  try {
    return await work()
  } catch {
    return undefined
  }
}
export async function refreshApp() {
  if (!desktop) return
  const v = await guard(() => call<AppSnapshot>('app_snapshot'))
  if (v) {
    state.app = v
    langSetting.value = v.settings.lang
    tzSetting.value = v.settings.timezone
    state.active = v.active
  }
}
export async function refreshPool() {
  if (!desktop) return
  const v = await guard(() => call('pool_snapshot'))
  if (v) state.pool = v
}
export async function refreshCatalog() {
  if (!desktop) return
  const v = await guard(() => call('catalog_snapshot'))
  if (v) state.catalog = v
}
export async function refreshLogs() {
  if (!desktop) return
  const v = await guard(() => call<{ logs: RequestLog[]; active: RequestLog[] }>('request_logs'))
  if (v) {
    state.logs = v.logs
    state.active = v.active
  }
}

/** Run a keyed action once, refresh state, and report the outcome. */
export async function act(key: string, work: () => Promise<unknown>, success?: string, refresh: Array<() => Promise<void>> = [refreshPool]) {
  if (state.pending.has(key)) return
  state.pending.add(key)
  try {
    const result: any = await work()
    if (result && typeof result === 'object' && result.ok === false) throw new Error(result.message || t('操作失败', 'Operation failed'))
    await Promise.all(refresh.map((r) => r()))
    if (success) Message.success(success)
    else if (result && typeof result === 'object' && typeof result.message === 'string' && result.ok) Message.success(result.message)
    return result
  } catch (e) {
    Message.error(String(e instanceof Error ? e.message : e))
  } finally {
    state.pending.delete(key)
  }
}
export const busy = (key: string) => state.pending.has(key)

export async function saveSettings(patch: Record<string, unknown>, success?: string) {
  return act('settings', () => call('save_settings', { patch }), success, [refreshApp])
}

export async function copy(text: string) {
  try {
    await navigator.clipboard.writeText(text)
    Message.success(t('已复制', 'Copied'))
  } catch {
    Message.error(t('系统不允许写入剪贴板，请手动选择复制', 'Clipboard unavailable; copy manually'))
  }
}

// Theme: light / dark / follow system.
const media = window.matchMedia?.('(prefers-color-scheme: dark)')
export const themeSetting = ref<'light' | 'dark' | 'system'>('light')
function applyTheme() {
  const dark = themeSetting.value === 'dark' || (themeSetting.value === 'system' && !!media?.matches)
  document.documentElement.dataset.theme = dark ? 'dark' : 'light'
  if (dark) document.body.setAttribute('arco-theme', 'dark')
  else document.body.removeAttribute('arco-theme')
}
media?.addEventListener?.('change', applyTheme)
watch(themeSetting, applyTheme, { immediate: true })
watch(
  () => state.app?.settings.theme,
  (v) => {
    if (v) themeSetting.value = v
  }
)

const desktop = isDesktop || previewMode
let started = false
export function startPolling() {
  if (started) return
  started = true
  setInterval(() => (state.now = Date.now()), 1000)
  const tick = () => {
    if (document.hidden) return
    refreshApp()
    if (['overview', 'accounts', 'access', 'checkin'].includes(page.value)) refreshPool()
    if (['overview', 'requests'].includes(page.value)) refreshLogs()
  }
  refreshApp().then(() => Promise.all([refreshPool(), refreshCatalog(), refreshLogs()]))
  setInterval(tick, 3000)
}
