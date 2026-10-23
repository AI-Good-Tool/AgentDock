// Development-only browser preview with synthetic data (no real accounts).
// Enabled by `npm run dev` + `?preview=1`; never bundled into desktop builds' code path.
import { version as appVersion } from '../../package.json'

const now = Date.now()
const day = 86400000
const nextDailyRun = new Date(now)
nextDailyRun.setHours(7, 0, 0, 0)
if (nextDailyRun.getTime() <= now) nextDailyRun.setDate(nextDailyRun.getDate() + 1)
const iso = (t: number) => new Date(t).toISOString()
let updateCancelled = false
const settings = {
  port: 24800,
  strategy: 'roundRobin',
  providers: {
    workbuddy: { enabled: true, regions: ['cn', 'intl'], concurrency: 4 },
    qoder: { enabled: true, regions: ['cn', 'intl'], concurrency: 4 },
    trae: { enabled: true, regions: ['cn', 'intl'], concurrency: 4 },
  },
  routes: {},
  pinned: {},
  rejectBrowserOrigin: true,
  autoStart: false,
  theme: new URLSearchParams(location.search).get('theme') || 'light',
  lang: new URLSearchParams(location.search).get('lang') || 'system',
  timezone: 'system',
  tokenFormat: 'km',
  checkin: { workbuddy: true, qoder: true, trae: true, onStartup: true, scheduled: true, intervalMinutes: 60, schedule: { kind: 'daily', every: 1, time: '07:00', weekdays: [1, 2, 3, 4, 5], date: '' } },
  logging: true,
  version: 3,
} as Record<string, any>
const checkinStatus = {
  trigger: 'startup',
  lastRun: now - 12 * 60e3,
  nextRun: nextDailyRun.getTime(),
  results: {
    workbuddy: [{ id: 'cn:1', name: 'wb-main', ok: true, message: '签到状态已确认' }],
    qoder: [{ name: 'Qoder 演示 01', edition: 'cn', phase: 'claimed', credits: 100, msg: '' }],
    trae: [],
  },
}
const appLogs = [
  { seq: 102, time: now - 10e3, level: 'info', source: 'trae', message: '[refresh] Trae CN 用量刷新完成' },
  { seq: 101, time: now - 12e3, level: 'info', source: 'trae', message: '[checkin] 签到状态已确认' },
  { seq: 100, time: now - 15e3, level: 'warn', source: 'trae', message: '[error] 自动签到：签到未完成，请检查凭证或官方签到开放状态' },
  { seq: 99, time: now - 20e3, level: 'info', source: 'gateway', message: 'GLM-5.3-Flash workbuddy → 200 · 2310 ms', fields: { account: 'wb-main', region: 'cn', realModel: 'glm-5.3-flash', outcome: 'success' } },
  { seq: 98, time: now - 22e3, level: 'info', source: 'workbuddy.agent', message: 'main Agent · conversation → HTTP 200', fields: { account: 'wb-main', region: 'cn', model: 'glm-5.3-flash', purpose: 'conversation', agentType: 'main', deviceSdk: true, firstByteMs: 640 } },
  { seq: 97, time: now - 23e3, level: 'debug', source: 'workbuddy.sdk', message: '官方传输已转发对话请求', fields: { status: 200, deviceSdk: true, firstByteMs: 610 } },
  { seq: 96, time: now - 90e3, level: 'warn', source: 'workbuddy.agent', message: '已拒绝 Agent 请求：客户端辅助 Agent 请求已在接入设置中禁用', fields: { purpose: 'conversation_topic' } },
  { seq: 95, time: now - 12 * 60e3, level: 'info', source: 'checkin.qoder', message: 'qoder-dev：', fields: { phase: 'claimed', credits: 100 } },
  { seq: 94, time: now - 12 * 60e3 - 2e3, level: 'info', source: 'checkin', message: '开始签到（启动后自动执行）', fields: { workbuddy: true, qoder: true, trae: true } },
  { seq: 93, time: now - 13 * 60e3, level: 'info', source: 'workbuddy.sdk', message: '设备 SDK 已取得设备令牌', fields: { status: 204, durationMs: 820 } },
  { seq: 92, time: now - 13 * 60e3 - 1e3, level: 'info', source: 'workbuddy.sdk', message: '官方传输进程已启动' },
  { seq: 91, time: now - 13 * 60e3 - 2e3, level: 'info', source: 'gateway', message: '网关已启动：127.0.0.1:24800', fields: { strategy: 'roundRobin' } },
  { seq: 90, time: now - 14 * 60e3, level: 'error', source: 'gateway', message: 'kimi-k3 trae → 502 · 30120 ms', fields: { account: 'trae-cn', outcome: 'upstream_error', message: '上游流中断' } },
]
const target = (provider: string, region: string, id: string, name: string, extra: any = {}) => ({
  key: `${provider}:${region}:${id}`,
  provider,
  region,
  id,
  name,
  supportsTools: true,
  supportsImages: provider === 'workbuddy' || provider === 'trae',
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
  accountName: { workbuddy: 'WorkBuddy 演示 01', qoder: 'Qoder 演示 01', trae: 'Trae 演示 01' }[provider],
  status,
  outcome,
  durationMs: 1200 + i * 310,
  firstByteMs: 420,
  usage: outcome === 'completed' ? { prompt_tokens: 12000 + i * 900, completion_tokens: 640 + i * 40, prompt_tokens_details: { cached_tokens: 8000 } } : {},
  message: outcome === 'completed' ? '' : 'Qoder 上游拒绝请求（HTTP 429）',
  upstreamCode: outcome === 'completed' ? null : 'rate_limited',
  strategy: 'RoundRobin',
  stream: i % 2 === 0,
  details: { protocol: 'anthropic-messages', method: 'POST', clientPath: '/v1/messages', startedAt: iso(now - i * 7 * 60000 - 2000), requestSummary: { messageCount: 6, toolCount: 3 }, upstreamSummary: provider === 'trae' ? { transport: 'trae-remote-agent', cloudSessionCleanup: 'deleted' } : { transport: 'legacy' }, error: {}, responseBytes: 18234, streamComplete: true, finishReason: 'stop', upstreamStatus: status },
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
  presentation: { uid: 'demo-****', nickname: name, displayName: name, phoneNumber: id === 'cn:a1' ? '+86 153 0000 0000' : '15300000000', clients: { workbuddy: id === 'cn:a1' } },
  credit: { ok: true, totalCapacity: remaining + 600, totalRemaining: remaining, updatedAt: now - 3600e3, resources: [{ packageName: '平台奖励积分', total: 100, used: 2.9, remaining: 97.1, expireAt: now + 2 * day }, { packageCode: 'TCACA_code_002_AkiJS3ZHF5', total: 1500, used: 500, remaining: 1000, expireAt: now + 20 * day }] },
  checkin: { today_checked_in: variant === 'cn', streak_days: 5, today_credit: 10, daily_credit: 10, active: true },
  checkinUpdatedAt: id === 'cn:a2' ? now - 2 * day : now - 3600e3,
  expiresAt: now + 20 * day,
})
const runtime: Record<string, any> = {}
const addRt = (provider: string, id: string, name: string, region: string, credit: number | null, unit = 'credits', extra: any = {}) =>
  (runtime[`${provider}/${id}`] = { inFlight: 0, concurrency: 4, authFailed: false, enabled: true, credit: { remaining: credit, unit, earliestExpiry: now + 2 * day }, expiresAt: now + 10 * day, name, region, provider, id, ...extra })
addRt('workbuddy', 'cn:a1', 'WorkBuddy 演示 01', 'cn', 3197)
addRt('workbuddy', 'cn:a2', 'WorkBuddy 演示 02', 'cn', 5666)
addRt('workbuddy', 'ai:a3', 'WorkBuddy 演示 03', 'intl', 820)
addRt('qoder', 'cn:q1', 'Qoder 演示 01', 'cn', 1450, 'credits', { inFlight: 1 })
addRt('qoder', 'intl:q2', 'Qoder 演示 02', 'intl', 300, 'credits', { cooldownUntil: now + 120000 })
addRt('trae', 't1', 'Trae 演示 01', 'cn', 2774.65)
addRt('trae', 't2', 'Trae 演示 02', 'intl', null, 'usd')
const pool = {
  runtime,
  workbuddy: {
    accounts: [wbAccount('cn:a1', 'WorkBuddy 演示 01', 'cn', 3197.08), wbAccount('cn:a2', 'WorkBuddy 演示 02', 'cn', 5666.57), wbAccount('ai:a3', 'WorkBuddy 演示 03', 'ai', 820, false)],
    clients: { cli: { available: true }, cnIde: { available: true }, aiIde: { available: false }, vscode: { available: true } },
    settings: { accountRefreshEnabled: false, accountRefreshMinutes: 30, allowAuxiliaryAgents: true, useDeviceSdk: false, tokenAutoRenew: true },
    accountRefresh: { running: false, success: 0, failed: 0 },
    autoCheckin: true,
    checkinConfig: { start: '09:00', end: '24:00' },
    checkinRecords: [{ at: now - 3600e3, accountId: 'cn:a1', submit: true, status: { today_checked_in: true }, error: null }],
    version: '5.8.3',
  },
  qoder: {
    editions: { cn: { label: 'Qoder CN 国内版', installed: true, running: false, currentUid: 'u1', currentName: 'Qoder 演示 01' }, intl: { label: 'Qoder 国际版', installed: true, running: true, currentUid: '', currentName: '' } },
    accounts: [
      { id: 'cn:q1', edition: 'cn', name: 'Qoder 演示 01', email: 'de***@example.com', phone: '', note: '', uid: 'demo-****', expiresAt: iso(now + 10 * day), isCurrent: true, hasToken: true, enabled: true, credit: { ok: true, cached: true, updatedAt: now - 600e3, usage: { userQuota: { remaining: 1300, total: 2000, used: 700 }, addOnQuota: { remaining: 150, total: 300, used: 150 }, expiresAt: now + 25 * day } } },
      { id: 'intl:q2', edition: 'intl', name: 'Qoder 演示 02', email: '', phone: '138****0000', note: '', uid: 'demo-****', expiresAt: iso(now + 2 * 3600e3), isCurrent: false, hasToken: true, enabled: true, credit: null },
    ],
    settings: { autoClaim: true },
    scheduler: { lastRun: '2026-10-21 10:05:12', lastResults: [] },
    claimLog: [{ time: '2026-10-21 10:05:12', results: [{ name: 'Qoder 演示 01', edition: 'cn', phase: 'claimed', msg: '领取成功 +100 Credits', credits: 100 }, { name: 'Qoder 演示 02', edition: 'intl', phase: 'nothing', msg: '今日无可领积分', credits: 0 }] }],
    gateway: { transport: 'legacy', cnEndpoint: 'https://gateway.qoder.com.cn', intlEndpoint: 'https://api2.qoder.sh', disabledAccounts: [] },
  },
  trae: {
    accounts: [
      { id: 't1', variant: 'cn', username: 'Trae 演示 01', email: 'tr***@example.com', mobile: '138****0000', note: '', plan: 'Pro', expiredAt: iso(now + 12 * day), tokenExpired: false, snapshot: { time: iso(now - 1800e3) }, balance: { remaining: 2774.65, total: 4600, used: 1825.35, unit: 'credits' }, creditPacks: [{ name: '平台奖励积分', total: 100, used: 25.35, remaining: 74.65, expiresAt: iso(now + 7 * day), active: true }, { name: '版本基础用量', total: 4500, used: 1800, remaining: 2700, expiresAt: iso(now + 20 * day), active: true }], autoCheckin: true, checkin: { checked_in: true, credits: 100, time: iso(now - 7200e3) }, checkedToday: true, enabled: true, models: [{ id: 'cfg-k3', model: 'kimi-k3', name: 'Kimi-K3' }] },
      { id: 't2', variant: 'intl', username: 'Trae 演示 02', email: 'in***@example.com', note: '', plan: 'Free', expiredAt: iso(now - day), tokenExpired: true, snapshot: null, balance: { remaining: 4.5, total: 10, used: 5.5, unit: 'usd' }, creditPacks: [], autoCheckin: false, checkin: null, checkedToday: false, enabled: true, models: [] },
    ],
    events: [{ id: 'e1', time: iso(now - 7200e3), type: 'checkin', message: '自动签到成功' }],
    checkinHistory: [{ accountId: 't1', accountName: 'Trae 演示 01', time: iso(now - 7200e3), source: 'auto', status: 'success', credits: 100 }],
    clients: [{ variant: 'cn', name: 'Trae CN', installed: true, version: '3.4.4' }, { variant: 'intl', name: 'Trae', installed: true, version: '3.6.4' }],
    logins: [],
    busy: [],
  },
}
// Opt-in visual fixtures: expired login, invalid login without an expiry date,
// and a valid account remain distinguishable without touching real accounts.
if (new URLSearchParams(location.search).has('expired')) {
  const wb = pool.workbuddy.accounts[0]
  wb.expiresAt = now - day
  wb.enabled = false
  Object.assign(wb.credit, { stale: true })
  Object.assign(runtime['workbuddy/cn:a1'], { enabled: false, authFailed: true, expiresAt: wb.expiresAt })
  const qoder = pool.qoder.accounts[0]
  qoder.expiresAt = iso(now - day)
  qoder.enabled = false
  Object.assign(runtime['qoder/cn:q1'], { enabled: false, authFailed: true, expiresAt: now - day })
  const trae = pool.trae.accounts[0]
  trae.enabled = false
  Object.assign(runtime['trae/t1'], { enabled: false, authFailed: true })
}
if (new URLSearchParams(location.search).has('stale-auth')) {
  const wb = pool.workbuddy.accounts[0]
  wb.enabled = false
  Object.assign(wb.credit, { stale: true, authFailed: true, error: '服务端返回 HTML 错误页：401 Authorization Required' })
  runtime['workbuddy/cn:a1'].enabled = false
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
    accounts: [group('WorkBuddy 演示 01', 300000), group('Qoder 演示 01', 252345)],
    requests: logs.map((l) => ({ ...l, timestamp: Date.parse(l.time), usageKnown: l.outcome === 'completed', input: 12000, output: 640, cacheRead: 8000 })),
  }
}
export function preview(command: string, args?: Record<string, any>): unknown {
  switch (command) {
    case 'app_snapshot':
      return { version: appVersion, dataDir: '/Users/****/.agentdock', running: true, port: 24800, apiKey: 'ad-preview0000000000000000000000000000', error: '', settings: structuredClone(settings), active: [], checkin: checkinStatus, migration: { workbuddy: { files: 1, usageRows: 3262 }, qoder: { files: 31, usageRows: 0 }, trae: { files: 4, requests: 0 } } }
    case 'save_settings':
      for (const [k, v] of Object.entries(args?.patch ?? {})) settings[k] = k === 'checkin' ? { ...settings.checkin, ...(v as object) } : v
      return settings
    case 'app_logs': {
      const f = args?.filter ?? {}
      const rank: Record<string, number> = { debug: 0, info: 1, warn: 2, error: 3 }
      const entries = appLogs.filter((e) => rank[e.level] >= rank[f.level || 'debug'] && (!f.source || e.source === f.source || e.source.startsWith(f.source + '.')) && (!f.query || JSON.stringify(e).toLowerCase().includes(String(f.query).toLowerCase())))
      return { entries: settings.logging ? entries : [], total: appLogs.length, sources: [...new Set(appLogs.map((e) => e.source))].sort(), file: '/Users/me/.agentdock/logs/app.log', enabled: settings.logging }
    }
    case 'backup_export':
      return { path: '/Users/me/Downloads/agentdock-backup-20261021-101500.json', summary: {} }
    case 'backup_inspect':
      return { path: args?.path, summary: { exportedAt: iso(now - day), app: '1.0.0', agentdockSettings: true, workbuddy: { present: true, accounts: 3, settings: true }, qoder: { present: true, accounts: 2, settings: true }, trae: { present: true, accounts: 1, settings: true } } }
    case 'backup_choose':
      return { path: '/Users/****/Downloads/agentdock-backup-demo.json', summary: { exportedAt: iso(now - day), app: appVersion, agentdockSettings: true, workbuddy: { present: true, accounts: 3, settings: true }, qoder: { present: true, accounts: 2, settings: true }, trae: { present: true, accounts: 1, settings: true } } }
    case 'backup_import':
      return { workbuddy: { accounts: 3, settings: true }, qoder: { accounts: 2, settings: true }, trae: { accounts: 1, settings: true } }
    case 'wb_set_runtime_settings':
      Object.assign(pool.workbuddy.settings, args?.settings)
      return { ok: true }
    case 'wb_oauth_start':
      return { loginId: args?.variant === 'ai' ? 'preview:ai:a3' : 'preview:cn:a1', verificationUri: 'https://example.com/preview-oauth' }
    case 'wb_open_oauth_link':
      return { ok: true }
    case 'wb_renew_token':
    case 'wb_import_accounts':
    case 'wb_oauth_poll': {
      const id = command === 'wb_renew_token' ? String(args?.id) : command === 'wb_import_accounts' ? 'cn:a1' : new URLSearchParams(location.search).get('reauth') === 'wrong-account' ? 'cn:a2' : String(args?.loginId || '').replace(/^preview:/, '')
      const a = pool.workbuddy.accounts.find(a => a.id === id)
      if (!a) return { done: true, error: '演示账号不存在' }
      a.expiresAt = Date.now() + 20 * day
      a.authFailed = false
      Object.assign(a.credit, { ok: true, authFailed: false, stale: false, error: undefined, updatedAt: Date.now() })
      Object.assign(runtime[`workbuddy/${id}`], { authFailed: false, expiresAt: a.expiresAt })
      if (command === 'wb_renew_token') return { ok: true, message: 'Token 已续期，登录状态已刷新' }
      return command === 'wb_import_accounts' ? { count: 1, accountId: id, updated: true } : { done: true, added: true, accountId: id }
    }
    case 'wb_session_accounts':
      return { variant: args?.variant, current: 'u1', running: args?.variant === 'cn', accounts: args?.variant === 'cn' ? [{ uid: 'u1', name: 'wb-main', current: true, inLibrary: true }, { uid: 'u2', name: 'wb-backup', current: false, inLibrary: true }] : [{ uid: 'u9', name: 'wb-global', current: false, inLibrary: true }] }
    case 'wb_list_sessions':
      return { sessions: args?.uid === 'u1' ? [{ id: 'w1', title: '重构签到调度', cwd: '/Users/me/projects/agentdock', updatedAt: now - 3600e3, hasHistory: true, isPlayground: false }, { id: 'w2', title: '每日周报整理', cwd: '/Users/me/WorkBuddy/playground', updatedAt: now - day, hasHistory: true, isPlayground: true }, { id: 'w3', title: '分析短信未推送原因', cwd: '/Users/me/projects/sms-forwarding', updatedAt: now - 3 * day, hasHistory: false, isPlayground: false }] : [] }
    case 'wb_session_groups':
      return { storeStatus: 'ready', groups: [{ groupId: 'g1', title: '重构签到调度', projectLabel: 'agentdock', accountNames: ['wb-main', 'wb-backup'], memberCount: 2, latestActivityAt: now - 3600e3, summaryStatus: 'behind', hasSafeSource: true }, { groupId: 'g2', title: 'Chrome 调试排查问题', projectLabel: 'hssmis-web', accountNames: ['wb-main', 'wb-global'], memberCount: 2, latestActivityAt: now - 2 * day, summaryStatus: 'latest', hasSafeSource: true }] }
    case 'wb_session_group_detail':
      return { members: [{ memberId: 'm1', accountName: 'wb-main', variant: 'cn', title: '重构签到调度', versionStatus: 'latest', recordCount: 128, updatedAt: now - 3600e3, linkState: 'active' }, { memberId: 'm2', accountName: 'wb-backup', variant: 'cn', title: '重构签到调度', versionStatus: 'behind', recordCount: 96, updatedAt: now - day, linkState: 'active' }] }
    case 'account_json':
      return { format: 'agentdock-account', version: 1, provider: args?.provider, region: 'cn', exportedAt: iso(now), account: { id: 'c7d1-preview', uid: '07efe4fd-a82c-4270-a2f2-27b1b16f0c11', nickname: '大飞龙', access_token: 'eyJhbGciOiJIUzI1NiJ9.preview.signature', refresh_token: 'rt-preview-0000000000', domain: 'www.workbuddy.cn', variant: 'cn', expiresAt: now + 20 * day, enterpriseId: null, verified: true, plan: { name: 'Pro', credits: 1200.5 } } }
    case 'account_json_import':
      return { ok: true, count: args?.json?.accounts?.length ?? (Array.isArray(args?.json) ? args.json.length : 1), message: '演示：账号 JSON 已导入' }
    case 'account_json_save_batch':
      return { ok: true, count: args?.accounts?.length ?? 0, path: '/preview/Downloads/agentdock-accounts.json', message: '演示：已导出所选账号 JSON' }
    case 'app_update_check':
      return { currentVersion: appVersion, latestVersion: '1.1.0', releaseUrl: 'https://github.com/AI-Good-Tool/AgentDock/releases', target: 'aarch64-apple-darwin', assetName: 'AgentDock_1.1.0_aarch64-apple-darwin.dmg', assetSize: 8071168, notes: '- Windows 支持\n- 关于页面与在线更新', hasUpdate: true, canDownload: true, reason: '已找到匹配平台且提供 SHA256 的安装包' }
    case 'app_update_download':
      updateCancelled = false
      return new Promise((resolve, reject) => setTimeout(() => {
        if (updateCancelled) return reject(new Error('下载已取消'))
        if (new URLSearchParams(location.search).get('update') === 'download-error') return reject(new Error('预览：模拟下载校验失败'))
        resolve({ version: '1.1.0', path: '/preview/AgentDock.dmg', sha256: 'a'.repeat(64), sizeBytes: 8071168 })
      }, new URLSearchParams(location.search).get('update') === 'slow' ? 10000 : 1500))
    case 'app_update_cancel_download':
      updateCancelled = true
      return null
    case 'app_update_install':
      if (new URLSearchParams(location.search).get('update') === 'install-error') throw new Error('预览：模拟安装目录不可写')
      return null
    case 'checkin_run_now':
      return checkinStatus
    case 'pool_snapshot':
      return structuredClone(pool)
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
    case 'qoder_credits': {
      const account = pool.qoder.accounts.find((account) => account.id === args?.id)
      if (new URLSearchParams(location.search).has('credit-error')) throw new Error('演示：网络暂时不可用')
      return { ok: !!account?.credit, data: { usage: account?.credit?.usage }, message: account?.credit ? '' : '演示账号尚未查询' }
    }
    case 'qoder_credit_stats':
      return { accounts: [{ id: 'cn:q1', edition: 'cn', name: 'Qoder 演示 01', ok: true, quotaTotal: 2300, quotaUsed: 850, totalCredits: 1200, peakCredits: 140, peakDate: '2026-10-18', heat: Array.from({ length: 30 }, (_, i) => ({ date: iso(now - (29 - i) * day).slice(0, 10), value: (i * 37) % 120 })) }] }
    case 'wb_credit_statistics':
      return { accounts: [], events: [], officialUsage: { status: 'unavailable', accounts: [], requests: [] } }
    case 'wb_token_statistics':
      return { sources: [] }
    default:
      return { ok: true }
  }
}
