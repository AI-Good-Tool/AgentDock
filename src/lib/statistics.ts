import { t } from '../i18n'
import { inRange as inDayRange, type RangeKey } from './range'
import type { TokenStatsGroup, TokenStatsTotals } from '../statistics-types'
export type StatsRange = RangeKey
export type CustomRange = [string, string] | null
export const sourceNames = (): Record<string, string> => ({
  workbuddy: t('WorkBuddy 国内版', 'WorkBuddy China'),
  'workbuddy-ai': t('WorkBuddy 国际版', 'WorkBuddy International'),
  'codebuddy-cli': 'CodeBuddy CLI',
  'codebuddy-ide': 'CodeBuddy IDE',
  proxy: t('本地反代（WorkBuddy）', 'Local proxy (WorkBuddy)'),
})
export const emptyTotals = (): TokenStatsTotals => ({
  total: 0,
  input: 0,
  output: 0,
  cacheRead: 0,
  cacheWrite: 0,
  uncachedInput: 0,
  records: 0,
  cacheHitRate: null,
})
export const exact = (v: number | null | undefined) =>
  v == null || !Number.isFinite(v)
    ? '—'
    : new Intl.NumberFormat('zh-CN', { maximumFractionDigits: 2 }).format(v)
export const compact = (v: number) =>
  new Intl.NumberFormat('en-US', { notation: 'compact', maximumFractionDigits: 1 })
    .format(v)
    .toUpperCase()
export function dateKey(at: number = Date.now()) {
  const p = new Intl.DateTimeFormat('en-CA', {
    timeZone: 'Asia/Shanghai',
    year: 'numeric',
    month: '2-digit',
    day: '2-digit',
  }).formatToParts(at)
  const get = (key: string) => p.find((p) => p.type === key)?.value
  return `${get('year')}-${get('month')}-${get('day')}`
}
export function shiftDay(day: string, offset: number) {
  return new Date(Date.parse(`${day}T12:00:00Z`) + offset * 86400000).toISOString().slice(0, 10)
}
/** Official WorkBuddy days are Shanghai dates. */
export function inRange(day: string, range: StatsRange, custom?: CustomRange) {
  return inDayRange(day, range, custom, dateKey())
}
export function sumTokens(rows: Partial<TokenStatsTotals>[]): TokenStatsTotals {
  const out = emptyTotals()
  for (const r of rows)
    for (const key of ['input', 'output', 'cacheRead', 'cacheWrite', 'records'] as const)
      out[key] += r[key] ?? 0
  out.uncachedInput = Math.max(0, out.input - out.cacheRead)
  out.total = out.input + out.output + out.cacheWrite
  out.cacheHitRate = out.input > 0 ? out.cacheRead / out.input : null
  return out
}
export function periodTokens(daily: TokenStatsGroup[], range: StatsRange, custom?: CustomRange) {
  return sumTokens(daily.filter((p) => inRange(p.key, range, custom)))
}
export function requestDate(value: string | number) {
  if (
    typeof value === 'string' &&
    /^\d{4}-\d{2}-\d{2}/.test(value) &&
    !/[zZ]|[+-]\d{2}:\d{2}$/.test(value)
  )
    return value.slice(0, 10)
  const n = Number(value)
  const at = Number.isFinite(n) ? (n < 1e11 ? n * 1000 : n) : Date.parse(String(value))
  return Number.isFinite(at) ? dateKey(at) : ''
}
export function heatmapDays(daily: TokenStatsGroup[]) {
  const values = new Map(daily.map((p) => [p.key, p.total]))
  const today = dateKey()
  return Array.from({ length: 371 }, (_, i) => {
    const day = shiftDay(today, i - 370)
    return { day, total: values.get(day) ?? 0 }
  })
}
