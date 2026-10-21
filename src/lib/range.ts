// Date ranges evaluated in the display time zone.
import { t, dayKey, zoneOffset } from '../i18n'

export type RangeKey = 'today' | 'yesterday' | '7d' | '30d' | 'month' | 'lastmonth' | 'year' | 'all' | 'custom'
export const RANGE_KEYS: RangeKey[] = ['today', 'yesterday', '7d', '30d', 'month', 'lastmonth', 'year', 'all']
export function rangeLabel(k: RangeKey) {
  const labels: Record<RangeKey, [string, string]> = {
    today: ['今天', 'Today'],
    yesterday: ['昨天', 'Yesterday'],
    '7d': ['近 7 天', 'Last 7 days'],
    '30d': ['近 30 天', 'Last 30 days'],
    month: ['当月', 'This month'],
    lastmonth: ['上月', 'Last month'],
    year: ['今年', 'This year'],
    all: ['全部', 'All'],
    custom: ['自定义', 'Custom'],
  }
  return t(...labels[k])
}
const shift = (key: string, days: number) => {
  const d = new Date(key + 'T00:00:00Z')
  d.setUTCDate(d.getUTCDate() + days)
  return d.toISOString().slice(0, 10)
}
/** Inclusive [from, to] day keys; null = unbounded. */
export function rangeDays(k: RangeKey, custom?: [string, string] | null): [string | null, string | null] {
  const today = dayKey(Date.now())
  switch (k) {
    case 'today':
      return [today, today]
    case 'yesterday':
      return [shift(today, -1), shift(today, -1)]
    case '7d':
      return [shift(today, -6), today]
    case '30d':
      return [shift(today, -29), today]
    case 'month':
      return [today.slice(0, 8) + '01', today]
    case 'lastmonth': {
      const first = today.slice(0, 8) + '01'
      const end = shift(first, -1)
      return [end.slice(0, 8) + '01', end]
    }
    case 'year':
      return [today.slice(0, 5) + '01-01', today]
    case 'custom':
      return custom ? [custom[0], custom[1]] : [null, null]
    default:
      return [null, null]
  }
}
export function inRange(key: string, k: RangeKey, custom?: [string, string] | null) {
  const [from, to] = rangeDays(k, custom)
  return (!from || key >= from) && (!to || key <= to)
}
/** Millisecond bounds [from, to) for a day range in the display zone. */
export function rangeMillis(k: RangeKey, custom?: [string, string] | null): { from?: number; to?: number } {
  const [from, to] = rangeDays(k, custom)
  const at = (key: string) => {
    const utc = Date.parse(key + 'T00:00:00Z')
    return utc - zoneOffset(undefined, new Date(utc)) * 60000
  }
  return { from: from ? at(from) : undefined, to: to ? at(shift(to, 1)) : undefined }
}
/** Every day key of a range, for chart axes (bounded). */
export function daysOf(k: RangeKey, custom: [string, string] | null | undefined, data: string[]): string[] {
  let [from, to] = rangeDays(k, custom)
  if (!from) from = data.slice().sort()[0] ?? dayKey(Date.now())
  if (!to) to = dayKey(Date.now())
  const out = []
  for (let d = from; d <= to && out.length < 400; d = shift(d, 1)) out.push(d)
  return out
}
