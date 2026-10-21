// Statistics contract adapted from the MIT WorkBuddy Switch core.
export type WbVariant = 'cn' | 'ai'
export interface CreditStatsSummary {
  currentRemaining: number
  currentCapacity: number
  usageToday: number
  usage7Days: number
  usageThisMonth: number
  todayCheckedInAccounts: number
  todaySuccess: number
  todayAlready: number
  todayFailed: number
}

export interface CreditStatsDailyPoint {
  date: string
  usage: number
  /** 官方用量按模型聚合（全量，不受请求明细条数限制）；本地观察口径下为空 */
  models?: { model: string; requestCount: number; credit: number }[]
}

export interface CreditStatsAccount {
  accountId: string
  accountName: string
  isCurrent: boolean
  currentRemaining: number | null
  totalCapacity: number | null
  lastSnapshotAt: number | null
  usageToday: number
  usage7Days: number
  usageThisMonth: number
  checkedInToday: boolean | null
  checkinStatusToday: string | null
  lastCheckinAt: number | null
  lastCheckinResult: string | null
  /** 按账号的逐日观察消耗（缺省兼容旧后端）；官方可用时趋势图优先使用官方 daily */
  daily?: CreditStatsDailyPoint[]
  /** 档位标记。后端当前不下发，前端容忍性读取；缺省时回退到按 accountId 的映射表 */
  variant?: WbVariant
}

export interface CreditStatsUsageEvent {
  kind: 'usage'
  ts: number
  date: string
  accountId: string
  accountName: string
  amount: number
  /** 档位标记。后端当前不下发，前端容忍性读取；缺省时回退到按 accountId 的映射表 */
  variant?: WbVariant
}

export interface CreditStatsCheckinEvent {
  kind: 'checkin'
  ts: number
  date: string
  accountId: string | null
  accountName: string
  result: string
  error?: string | null
  /** 档位标记。后端当前不下发，前端容忍性读取；缺省时回退到按 accountId 的映射表 */
  variant?: WbVariant
}

export type CreditStatsEvent = CreditStatsUsageEvent | CreditStatsCheckinEvent

export type CreditOfficialUsageStatus = 'complete' | 'partial' | 'unavailable'

export interface CreditOfficialUsageSummary {
  usageToday: number
  usage7Days: number
  usageThisMonth: number
}

export interface CreditOfficialUsageModel {
  model: string
  requestCount: number
  credit: number
}

export interface CreditOfficialUsageAccount {
  accountId: string
  accountName: string
  ok: boolean
  requestCount: number
  detailTruncated: boolean
  usageToday: number | null
  usage7Days: number | null
  usageThisMonth: number | null
  error?: string | null
  reportedTotal?: number | null
  fetchedCount?: number
  /** 缺省兼容旧后端响应。 */
  models?: CreditOfficialUsageModel[]
  /** 按账号的逐日官方消耗（全量聚合，不受 requests 明细上限影响；缺省兼容旧后端） */
  daily?: CreditStatsDailyPoint[]
}

export interface CreditOfficialUsageRequest {
  accountId: string
  accountName: string
  requestId: string
  credit: number
  model: string
  client: string
  requestTime: string
}

export interface CreditOfficialUsageError {
  accountId: string
  accountName: string
  error: string
}

export interface CreditOfficialUsage {
  status: CreditOfficialUsageStatus
  rangeStart: string
  rangeEnd: string
  /** 官方用量最近一次采集时间；缓存命中时保持采集当时的时间。 */
  collectedAt?: number
  summary: CreditOfficialUsageSummary
  daily: CreditStatsDailyPoint[]
  accounts: CreditOfficialUsageAccount[]
  requests: CreditOfficialUsageRequest[]
  /** 官方全部有效请求按模型汇总；不受 requests 明细上限影响。 */
  models?: CreditOfficialUsageModel[]
  detailLimitPerAccount: number
  errors: CreditOfficialUsageError[]
}

export interface CreditStatistics {
  generatedAt: number
  retentionDays: number
  coverageStartAt: number | null
  summary: CreditStatsSummary
  daily: CreditStatsDailyPoint[]
  accounts: CreditStatsAccount[]
  events: CreditStatsEvent[]
  /** 官方接口不可用时仍使用上述本地观察字段；缺省兼容旧后端。 */
  officialUsage?: CreditOfficialUsage
}

export interface TokenStatsTotals {
  total: number
  input: number
  output: number
  cacheRead: number
  cacheWrite: number
  uncachedInput: number
  records: number
  cacheHitRate: number | null
}
export interface TokenStatsGroup extends TokenStatsTotals {
  key: string
  title?: string | null
  project?: string
  sessionId?: string
}
/** 一次模型调用的明细行；`total = input + output + cacheWrite`，`uncachedInput = max(0, input - cacheRead)`，`thinking` 是 `output` 中思考过程的 token 数（回复内容 = max(0, output - thinking)），均与聚合口径一致。 */
export interface TokenStatsRequestRow {
  timestamp: number
  model: string
  project: string
  sessionId: string
  title?: string | null
  input: number
  output: number
  cacheRead: number
  cacheWrite: number
  uncachedInput: number
  thinking: number
  total: number
}
/** `workbuddy-ai` 为国际版本地数据源，与国内版分开统计，数据源缺失时为空集。 */
export interface TokenStatsSource {
  source: 'workbuddy' | 'workbuddy-ai' | 'codebuddy-cli' | 'codebuddy-ide' | 'proxy'
  summary: TokenStatsTotals
  models: TokenStatsGroup[]
  projects: TokenStatsGroup[]
  sessions: TokenStatsGroup[]
  daily: TokenStatsGroup[]
  /** Optional model-specific daily series for trend filtering. */ dailyByModel?: Record<
    string,
    TokenStatsGroup[]
  >
  /** 本地反代与 CodeBuddy CLI 返回逐次明细；聚合来源或旧后端缺失时为空。 */ requests?: TokenStatsRequestRow[]
  hours: TokenStatsGroup[]
  filesScanned: number
  parseErrors: number
  coverageStartAt?: number | null
  coverageEndAt?: number | null
}
export interface TokenStatistics {
  generatedAt: number
  rangeDays?: number | null
  sources: TokenStatsSource[]
}
