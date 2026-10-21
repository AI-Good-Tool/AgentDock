// Development-only browser preview with synthetic data (no real accounts).
// Enabled by `npm run dev` + `?preview=1`; never bundled into desktop builds' code path.
const now = Date.now()
const day = 86400000
const iso = (t: number) => new Date(t).toISOString()
const settings = {
  port: 24800,
  strategy: 'roundRobin',
  providers: {
    workbuddy: { enabled: true, regions: ['cn', 'intl'], concurrency: 2 },
    qoder: { enabled: true, regions: ['cn', 'intl'], concurrency: 2 },
    trae: { enabled: true, regions: ['cn', 'intl'], concurrency: 2 },
  },
  routes: {},
  pinned: {},
  rejectBrowserOrigin: true,
  autoStart: false,
  theme: new URLSearchParams(location.search).get('theme') || 'light',
  lang: new URLSearchParams(location.search).get('lang') || 'system',
  timezone: 'system',
  tokenFormat: 'km',
}
const target = (provider: string, region: string, id: string, name: string, extra: any = {}) => ({
  key: `${provider}:${region}:${id}`,
  provider,
  region,
  id,
  name,
  supportsTools: provider !== 'trae',
  supportsImages: provider === 'workbuddy',
  efforts: provider === 'trae' ? [] : ['low', 'high'],
  contextWindow: 200000,
  maxOutput: 32000,
  ...extra,
})
const glm = [target('workbuddy', 'cn', 'glm-5.3-flash', 'GLM-5.3-Flash'), target('workbuddy', 'intl', 'glm-5.3-flash', 'GLM-5.3-Flash'), target('qoder', 'cn', 'gfmodel', 'GLM-5.3-Flash')]
const kimi = [target('workbuddy', 'cn', 'kimi-k3-1', 'Kimi-K3'), target('trae', 'cn', 'kimi-k3', 'Kimi-K3')]
const route = (routeId: string, alias: string, targets: any[], extra: any = {}) => ({
  routeId,
  alias,
  defaultAlias: alias,
  available: true,
  disabled: false,
  configured: false,
  custom: false,
  supportsTools: targets.every((t) => t.supportsTools),
  supportsImages: targets.every((t) => t.supportsImages),
  contextWindow: 200000,
  efforts: targets.every((t) => t.efforts.length) ? ['high'] : [],
  providers: [...new Set(targets.map((t) => t.provider))],
  regions: [...new Set(targets.map((t) => t.region))],
  targets,
  allTargets: targets,
  missing: [],
  ...extra,
})
const routes = [
  route('glm-5.3-flash', 'GLM-5.3-Flash', glm),
  route('kimi-k3', 'Kimi-K3', kimi),
  route('deepseek-flash', 'DeepSeek-Flash', [target('qoder', 'intl', 'dfmodel', 'DeepSeek-Flash')]),
  route('custom:1', 'team-coder', [target('qoder', 'cn', 'cdmodel', 'Qwen3-Coder')], { configured: true, custom: true, defaultAlias: 'Qwen3-Coder' }),
  route('trae:auto', 'auto', [target('trae', 'cn', 'auto', 'auto'), target('trae', 'intl', 'auto', 'auto')]),
]
const log = (i: number, provider: string, region: string, model: string, real: string, outcome = 'completed', status = 200) => ({
  id: 'req' + i,
  time: iso(now - i * 7 * 60000),
  protocol: ['anthropic-messages', 'openai-responses', 'openai-chat'][i % 3],
  path: '/v1/messages',
  requestedModel: model,
  model,
  routeId: model.toLowerCase(),
  realModel: real,
  modelName: model,
  provider,
  region,
  accountId: provider + '-' + region,
  accountName: { workbuddy: '码农 3 号', qoder: 'qoder-dev', trae: 'Trae 测试账号' }[provider],
  status,
  outcome,
  durationMs: 1200 + i * 310,
  firstByteMs: 420,
  usage: outcome === 'completed' ? { prompt_tokens: 12000 + i * 900, completion_tokens: 640 + i * 40, prompt_tokens_details: { cached_tokens: 8000 } } : {},
  message: outcome === 'completed' ? '' : 'Qoder 上游拒绝请求（HTTP 429）',
  upstreamCode: outcome === 'completed' ? null : 'rate_limited',
  strategy: 'RoundRobin',
  stream: i % 2 === 0,
  details: { protocol: 'anthropic-messages', method: 'POST', clientPath: '/v1/messages', startedAt: iso(now - i * 7 * 60000 - 2000), requestSummary: { messageCount: 6, toolCount: 3 }, upstreamSummary: { transport: 'legacy' }, error: {}, responseBytes: 18234, streamComplete: true, finishReason: 'stop', upstreamStatus: status },
})
const logs = [
  log(0, 'qoder', 'cn', 'GLM-5.3-Flash', 'gfmodel'),
  log(1, 'workbuddy', 'cn', 'GLM-5.3-Flash', 'glm-5.3-flash'),
  log(2, 'workbuddy', 'intl', 'GLM-5.3-Flash', 'glm-5.3-flash'),
  log(3, 'trae', 'cn', 'Kimi-K3', 'kimi-k3'),
  log(4, 'qoder', 'intl', 'DeepSeek-Flash', 'dfmodel', 'upstream_rejected', 429),
  log(5, 'workbuddy', 'cn', 'Kimi-K3', 'kimi-k3-1'),
]
const wbAccount = (id: string, name: string, variant: string, remaining: number, enabled = true) => ({
  id,
  name,
  source: 'switch',
  variant,
  enabled,
  checkinEnabled: variant === 'cn',
  note: '',
  displayField: '',
  inFlight: 0,
  authFailed: false,
  presentation: { uid: id.split(':')[1] + '-9f2e-4c1a', nickname: name, displayName: name, clients: { workbuddy: id === 'cn:a1' } },
  credit: { ok: true, totalCapacity: remaining + 600, totalRemaining: remaining, updatedAt: now - 3600e3, resources: [{ packageName: '平台奖励积分', total: 100, used: 2.9, remaining: 97.1, expireAt: now + 2 * day }, { packageCode: 'TCACA_code_002_AkiJS3ZHF5', total: 1500, used: 500, remaining: 1000, expireAt: now + 20 * day }] },
  checkin: { today_checked_in: variant === 'cn', streak_days: 5, today_credit: 10, daily_credit: 10, active: true },
  checkinUpdatedAt: now - 3600e3,
  expiresAt: now + 20 * day,
})
const runtime: Record<string, any> = {}
const addRt = (provider: string, id: string, name: string, region: string, credit: number | null, unit = 'credits', extra: any = {}) =>
  (runtime[`${provider}/${id}`] = { inFlight: 0, concurrency: 2, authFailed: false, enabled: true, credit: { remaining: credit, unit, earliestExpiry: now + 2 * day }, expiresAt: now + 10 * day, name, region, provider, id, ...extra })
addRt('workbuddy', 'cn:a1', '码农 3 号', 'cn', 3197)
addRt('workbuddy', 'cn:a2', '小米不是米', 'cn', 5666)
addRt('workbuddy', 'ai:a3', 'global-dev', 'intl', 820)
addRt('qoder', 'cn:q1', 'qoder-dev', 'cn', 1450, 'credits', { inFlight: 1 })
addRt('qoder', 'intl:q2', 'qoder-global', 'intl', 300, 'credits', { cooldownUntil: now + 120000 })
addRt('trae', 't1', 'Trae 测试账号', 'cn', 2774.65)
addRt('trae', 't2', 'trae-intl', 'intl', null, 'usd')
const pool = {
  runtime,
  workbuddy: {
    accounts: [wbAccount('cn:a1', '码农 3 号', 'cn', 3197.08), wbAccount('cn:a2', '小米不是米', 'cn', 5666.57), wbAccount('ai:a3', 'global-dev', 'ai', 820, false)],
    clients: { cli: { available: true }, cnIde: { available: true }, aiIde: { available: false }, vscode: { available: true } },
    settings: { accountRefreshEnabled: false, accountRefreshMinutes: 30, allowAuxiliaryAgents: true, useDeviceSdk: false },
    accountRefresh: { running: false, success: 0, failed: 0 },
    autoCheckin: true,
    checkinConfig: { start: '09:00', end: '24:00' },
    checkinRecords: [{ at: now - 3600e3, accountId: 'cn:a1', submit: true, status: { today_checked_in: true }, error: null }],
    version: '5.8.3',
  },
  qoder: {
    editions: { cn: { label: 'Qoder CN 国内版', installed: true, running: false, currentUid: 'u1', currentName: 'qoder-dev' }, intl: { label: 'Qoder 国际版', installed: true, running: true, currentUid: '', currentName: '' } },
    accounts: [
      { id: 'cn:q1', edition: 'cn', name: 'qoder-dev', email: 'dev@example.com', phone: '', note: '', uid: 'u1-7f3a', expiresAt: iso(now + 10 * day), isCurrent: true, hasToken: true, enabled: true, credit: { ok: true, updatedAt: now - 600e3, usage: { userQuota: { remaining: 1300, total: 2000, used: 700 }, addOnQuota: { remaining: 150, total: 300, used: 150 }, expiresAt: now + 25 * day } } },
      { id: 'intl:q2', edition: 'intl', name: 'qoder-global', email: '', phone: '13812345678', note: '', uid: 'u2-1b9c', expiresAt: iso(now + 2 * 3600e3), isCurrent: false, hasToken: true, enabled: true, credit: null },
    ],
    settings: { autoClaim: true },
    scheduler: { lastRun: '2026-10-21 10:05:12', lastResults: [] },
    claimLog: [{ time: '2026-10-21 10:05:12', results: [{ name: 'qoder-dev', edition: 'cn', phase: 'claimed', msg: '领取成功 +100 Credits', credits: 100 }, { name: 'qoder-global', edition: 'intl', phase: 'nothing', msg: '今日无可领积分', credits: 0 }] }],
    gateway: { transport: 'legacy', cnEndpoint: 'https://gateway.qoder.com.cn', intlEndpoint: 'https://api2.qoder.sh', disabledAccounts: [] },
  },
  trae: {
    accounts: [
      { id: 't1', variant: 'cn', username: 'Trae 测试账号', email: 'trae@example.com', note: '', plan: 'Pro', expiredAt: iso(now + 12 * day), tokenExpired: false, snapshot: { time: iso(now - 1800e3) }, balance: { remaining: 2774.65, total: 4600, used: 1825.35, unit: 'credits' }, creditPacks: [{ name: '平台奖励积分', total: 100, used: 25.35, remaining: 74.65, expiresAt: iso(now + 7 * day), active: true }, { name: '版本基础用量', total: 4500, used: 1800, remaining: 2700, expiresAt: iso(now + 20 * day), active: true }], autoCheckin: true, checkin: { checked_in: true, credits: 100, time: iso(now - 7200e3) }, checkedToday: true, enabled: true, models: [{ id: 'cfg-k3', model: 'kimi-k3', name: 'Kimi-K3' }] },
      { id: 't2', variant: 'intl', username: 'trae-intl', email: 'intl@example.com', note: '', plan: 'Free', expiredAt: iso(now - day), tokenExpired: true, snapshot: null, balance: { remaining: 4.5, total: 10, used: 5.5, unit: 'usd' }, creditPacks: [], autoCheckin: false, checkin: null, checkedToday: false, enabled: true, models: [] },
    ],
    events: [{ id: 'e1', time: iso(now - 7200e3), type: 'checkin', message: '自动签到成功' }],
    checkinHistory: [{ accountId: 't1', accountName: 'Trae 测试账号', time: iso(now - 7200e3), source: 'auto', status: 'success', credits: 100 }],
    clients: [{ variant: 'cn', name: 'Trae CN', installed: true, version: '3.4.4' }, { variant: 'intl', name: 'Trae', installed: true, version: '3.6.4' }],
    logins: [],
    busy: [],
  },
}
function stats() {
  const daily = Array.from({ length: 30 }, (_, i) => ({ key: iso(now - (29 - i) * day).slice(0, 10), total: Math.round(20000 + 15000 * Math.sin(i / 3) ** 2), records: 3 + (i % 5) }))
  const group = (key: string, total: number) => ({ key, total, records: Math.round(total / 9000), input: total * 0.9, output: total * 0.1 })
  return {
    summary: { total: 812345, input: 760000, output: 52345, thinking: 12000, cacheRead: 410000, cacheHitRate: 0.54, records: 96, success: 91, withUsage: 90 },
    daily,
    models: [group('GLM-5.3-Flash', 420000), group('Kimi-K3', 260000), group('DeepSeek-Flash', 132345)],
    realModels: [group('glm-5.3-flash', 300000), group('gfmodel', 120000), group('kimi-k3-1', 160000)],
    providers: [group('workbuddy', 460000), group('qoder', 252345), group('trae', 100000)],
    regions: [group('workbuddy:cn', 400000), group('qoder:cn', 200000), group('trae:cn', 100000)],
    accounts: [group('码农 3 号', 300000), group('qoder-dev', 252345)],
    requests: logs.map((l) => ({ ...l, timestamp: Date.parse(l.time), usageKnown: l.outcome === 'completed', input: 12000, output: 640, cacheRead: 8000 })),
  }
}
export function preview(command: string): unknown {
  switch (command) {
    case 'app_snapshot':
      return { version: '1.0.0', dataDir: '~/.agentdock', running: true, port: 24800, apiKey: 'ad-preview0000000000000000000000000000', error: '', settings, active: [], migration: { workbuddy: { files: 1, usageRows: 3262 }, qoder: { files: 31, usageRows: 0 }, trae: { files: 4, requests: 0 } } }
    case 'pool_snapshot':
      return pool
    case 'catalog_snapshot':
      return { routes, upstreams: routes.flatMap((r) => r.allTargets), status: { workbuddy: { cn: { count: 19 }, intl: { count: 12 } }, qoder: { cn: { count: 14, updated: iso(now - 3600e3) }, intl: { count: 11 } }, trae: { cn: { count: 6 }, intl: { count: 2 } } } }
    case 'request_logs':
      return { logs, active: [] }
    case 'proxy_statistics':
      return stats()
    case 'launchd_status':
      return { installed: false }
    case 'qoder_session_links':
      return { groups: [] }
    case 'qoder_list_sessions':
      return { total: 2, sessions: [{ id: 's1', edition: 'cn', title: '重构网关调度', cwd: '/Users/me/projects/agentdock', updatedAt: now - 3600e3, size: 182000 }, { id: 's2', edition: 'cn', title: 'Fix login flow', cwd: '/Users/me/projects/web', updatedAt: now - day, size: 42000, groupId: 'g1', contentState: 'consistent' }] }
    case 'qoder_credit_stats':
      return { accounts: [{ id: 'cn:q1', edition: 'cn', name: 'qoder-dev', ok: true, quotaTotal: 2300, quotaUsed: 850, totalCredits: 1200, peakCredits: 140, peakDate: '2026-10-18', heat: Array.from({ length: 30 }, (_, i) => ({ date: iso(now - (29 - i) * day).slice(0, 10), value: (i * 37) % 120 })) }] }
    case 'wb_credit_statistics':
      return { accounts: [], events: [], officialUsage: { status: 'unavailable', accounts: [], requests: [] } }
    case 'wb_token_statistics':
      return { sources: [] }
    default:
      return { ok: true }
  }
}
