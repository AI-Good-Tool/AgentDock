// WorkBuddy provider types (from WorkBuddy API).
export { call, desktop } from './api'
export interface CheckinConfig {
  start: string
  end: string
}
export interface CheckinRecord {
  at: number
  accountId: string
  submit: boolean
  status: Account['checkin'] | null
  error: string | null
  /** manual / startup / scheduled / launchd; absent in older records. */
  trigger?: string
}
export interface Account {
  id: string
  name: string
  source: string
  variant: string
  enabled: boolean
  /** Disabled by the user (from the AgentDock runtime; not stored by WorkBuddy). */
  disabled?: boolean
  checkinEnabled: boolean
  checkinUpdatedAt?: number
  checkinError?: string
  checkinErrorAt?: number
  checkinRunning?: boolean
  note?: string
  displayField?: string
  credit?: CreditExpiry | null
  modelLimits?: ModelLimit[]
  presentation?: {
    uid?: string
    nickname?: string
    displayName?: string
    email?: string
    phoneNumber?: string
    note?: string
    needsRelogin?: boolean
    travel?: { label: string; rewardCredit?: number; locationName?: string; arriveAt?: number }
    clients?: Record<string, boolean>
  }
  expiresAt?: number
  inFlight: number
  concurrency?: number
  managementInFlight?: number
  cooldownUntil?: number
  authFailed: boolean
  checkin?: {
    today_checked_in: boolean
    streak_days: number
    today_credit: number
    daily_credit: number
    active: boolean
  }
}
export interface RequestLog {
  realModel?: string
  id: string
  time: string
  model: string
  accountId: string
  accountName?: string
  accountVariant?: string
  status: number
  durationMs: number
  outcome: string
  details?: RequestDetails
}
export interface RequestDetails {
  protocol: string
  method: string
  clientPath: string
  startedAt: string
  upstreamUrl?: string | null
  upstreamStatus?: number | null
  upstreamRequestId?: string | null
  traceId?: string | null
  contentType?: string | null
  retryAfter?: string | null
  requestSummary: Record<string, unknown>
  upstreamSummary: Record<string, unknown>
  errorStage?: string | null
  error: {
    code?: string | number
    msg?: string
    message?: string
    displayMessage?: string
    type?: string
    param?: string
    requestId?: string
  }
  firstByteMs?: number | null
  responseBytes: number
  streamComplete: boolean
  finishReason?: string | null
  usage?: Record<string, unknown>
}
export interface Snapshot {
  accountPriority?: import('./lib/account-priority').AccountPriority
  settings?: RuntimeSettings
  accountRefresh?: {
    nextAt?: number
    lastAt?: number
    running: boolean
    success: number
    failed: number
  }
  officialProfile?: {
    desktopVersion: string
    cliVersion: string
    nodeVersion: string
    userAgent: string
    ideType: string
    ideVersion: string
    gzip: boolean
    transport?: string
    deviceToken?: string
  }
  accounts: Account[]
  running: boolean
  port: number
  apiKey: string
  logs: RequestLog[]
  version: string
  autoCheckin?: boolean
  checkinConfig?: CheckinConfig
  checkinRecords?: CheckinRecord[]
  catalogVariant?: string
  clients?: {
    cli?: ClientState
    cnIde?: ClientState
    aiIde?: ClientState
    vscode?: ClientState
    scannedAt?: number
  }
}
export interface RuntimeSettings {
  accountRefreshEnabled: boolean
  accountRefreshMinutes: number
  allowAuxiliaryAgents: boolean
  useDeviceSdk: boolean
  tokenAutoRenew: boolean
}
export interface ClientState {
  available: boolean
  migrationRequired?: boolean
}
export interface ModelLimit {
  model: string | null
  resetAt: number
  firstSeenAt: number
  source: string
}
export interface CreditResource {
  packageCode?: string
  packageName?: string
  total: number
  used: number
  remaining: number
  expireAt: number | null
}
export interface CreditExpiry {
  ok: boolean
  authFailed?: boolean
  totalCapacity?: number
  totalRemaining?: number
  expiringSoonRemaining?: number
  resources?: CreditResource[]
  updatedAt?: number
  stale?: boolean
  error?: string
}
export interface Model {
  id: string
  alias?: string
  regions?: string[]
  catalogRegions?: string[]
  routingRegions?: string[] | null
  name?: string
  credits?: string
  vendor?: string
  tags?: string[]
  disabled?: boolean
  supportsImages?: boolean
  supportsToolCall?: boolean
  supportsReasoning?: boolean
  onlyReasoning?: boolean
  maxInputTokens?: number
  maxOutputTokens?: number
  url?: string
}
