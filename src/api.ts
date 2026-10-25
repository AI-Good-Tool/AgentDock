import { invoke, isTauri } from '@tauri-apps/api/core'

export const desktop = isTauri()
/** Windows build (wording and OS integrations differ from macOS). */
export const isWindows = /Windows/i.test(navigator.userAgent)

export const previewMode = !desktop && import.meta.env.DEV && new URLSearchParams(location.search).has('preview')

export function call<T = any>(command: string, args?: Record<string, unknown>): Promise<T> {
  if (previewMode) return import('./dev/preview').then((m) => m.preview(command, args) as T)
  if (!desktop) return Promise.reject(new Error('请在 AgentDock 桌面应用中使用 / Please use the AgentDock desktop app'))
  return invoke<T>(command, args)
}

export type ProviderKey = 'workbuddy' | 'qoder' | 'trae'
export type Region = 'cn' | 'intl'
export type Strategy = 'roundRobin' | 'creditFirst' | 'expiryFirst' | 'ordered' | 'random'

export interface ProviderScope {
  enabled: boolean
  regions: Region[]
  concurrency: number
}
export interface RouteOverride {
  alias: string
  targets?: string[] | null
  disabled: boolean
  custom: boolean
  mergeInto?: string | null
}
export interface Settings {
  port: number
  strategy: Strategy
  providers: Record<ProviderKey, ProviderScope>
  routes: Record<string, RouteOverride>
  pinned: Record<string, string>
  rejectBrowserOrigin: boolean
  /** Images sent upstream per request, newest first; null sends all. */
  recentImages?: number | null
  autoStart: boolean
  theme: 'light' | 'dark' | 'system'
  lang: 'system' | 'zh' | 'en'
  timezone: string
  tokenFormat: string
  checkin: CheckinSettings
  logging: boolean
  version?: number
}
export interface CheckinSettings {
  workbuddy: boolean
  qoder: boolean
  trae: boolean
  onStartup: boolean
  scheduled: boolean
  intervalMinutes: number
  schedule?: CheckinSchedule | null
}
export interface CheckinSchedule {
  kind: 'minutes' | 'hours' | 'days' | 'daily' | 'date' | 'weekly'
  every: number
  time: string
  weekdays: number[]
  date: string
}
export interface CheckinStatus {
  running?: boolean
  trigger?: string
  lastRun?: number | null
  nextRun?: number | null
  results?: Record<ProviderKey, any[] | 'disabled'>
}
export interface AppLogEntry {
  seq: number
  time: number
  level: 'debug' | 'info' | 'warn' | 'error'
  source: string
  message: string
  fields?: any
}
export interface RequestLog {
  id: string
  time: string
  protocol: string
  path: string
  requestedModel: string
  model: string
  routeId: string
  realModel: string
  modelName: string
  provider: string
  region: string
  accountId: string
  accountName: string
  status: number
  outcome: string
  durationMs: number
  firstByteMs?: number | null
  usage?: Record<string, any> | null
  message: string
  upstreamCode?: string | null
  strategy: string
  stream: boolean
  /** 1-based account attempt within one client request. */
  attempt?: number
  /** Id of the first attempt; shared by every attempt of one request. */
  requestGroup?: string
  /** Local API key that made the request (empty for internal/legacy records). */
  keyId?: string
  keyHint?: string
  keyAlias?: string
  details?: any
}
export interface ApiKeyInfo {
  id: string
  key: string
  alias: string
  enabled: boolean
  /** null follows the global strategy. */
  strategy: Strategy | null
  /** Allowed regions per provider; null follows the global scope. */
  scope: Partial<Record<ProviderKey, Region[]>> | null
  createdAt: number
}
export interface AppSnapshot {
  version: string
  dataDir: string
  running: boolean
  port: number
  apiKey: string
  apiKeys?: ApiKeyInfo[]
  error: string
  settings: Settings
  active: RequestLog[]
  checkin?: CheckinStatus
}
export interface Target {
  key: string
  provider: ProviderKey
  region: Region
  id: string
  name: string
  supportsTools?: boolean | null
  supportsImages?: boolean | null
  efforts: string[]
  contextWindow?: number | null
  maxOutput?: number | null
}
export interface RouteView {
  routeId: string
  alias: string
  defaultAlias: string
  available: boolean
  disabled: boolean
  configured: boolean
  custom: boolean
  supportsTools: boolean
  supportsImages: boolean
  contextWindow?: number | null
  efforts: string[]
  providers: ProviderKey[]
  regions: Region[]
  targets: Target[]
  allTargets: Target[]
  missing: string[]
  /** Routes merged into this one; their upstreams are part of `allTargets`. */
  linked: { routeId: string; alias: string; targets: string[] }[]
}
export interface AccountRuntime {
  inFlight: number
  concurrency: number
  cooldownUntil?: number | null
  authFailed: boolean
  enabled: boolean
  credit: { remaining?: number | null; earliestExpiry?: number | null; unit: string; updatedAt?: number | null }
  expiresAt?: number | null
  blocked?: string | null
  name: string
  region: Region
  provider: ProviderKey
  id: string
}
