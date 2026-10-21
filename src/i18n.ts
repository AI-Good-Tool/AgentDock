// Language and time zone. Language follows the system unless chosen: any
// non-Chinese system language shows English. The time zone follows macOS
// unless the user picks one; turning "follow system" off defaults to Beijing
// for Chinese and US Eastern (New York) for English.
import { computed, ref, watch } from 'vue'
import { addI18nMessages, useLocale } from '@arco-design/web-vue'
import enUS from '@arco-design/web-vue/es/locale/lang/en-us'

export type LangSetting = 'system' | 'zh' | 'en'
export const langSetting = ref<LangSetting>('system')
export const tzSetting = ref<string>('system')

export function systemLang(): 'zh' | 'en' {
  const list = navigator.languages?.length ? navigator.languages : [navigator.language]
  return (list[0] || '').toLowerCase().startsWith('zh') ? 'zh' : 'en'
}
export const lang = computed<'zh' | 'en'>(() =>
  langSetting.value === 'system' ? systemLang() : langSetting.value
)
export const isZh = computed(() => lang.value === 'zh')

addI18nMessages({ 'en-US': enUS })
watch(lang, (v) => useLocale(v === 'zh' ? 'zh-CN' : 'en-US'), { immediate: true })

/** Inline bilingual text: t('中文', 'English'). */
export function t(zh: string, en: string): string {
  return lang.value === 'zh' ? zh : en
}

export function systemZone(): string {
  try {
    return Intl.DateTimeFormat().resolvedOptions().timeZone || 'UTC'
  } catch {
    return 'UTC'
  }
}
export function defaultZone(l: 'zh' | 'en' = lang.value): string {
  return l === 'zh' ? 'Asia/Shanghai' : 'America/New_York'
}
export const followSystemZone = computed(() => tzSetting.value === 'system')
export const zone = computed(() => (followSystemZone.value ? systemZone() : tzSetting.value))

/** Offset of a zone at an instant, in minutes east of UTC (DST aware). */
export function zoneOffset(tz = zone.value, at = new Date()): number {
  const parts = new Intl.DateTimeFormat('en-US', {
    timeZone: tz,
    hourCycle: 'h23',
    year: 'numeric',
    month: '2-digit',
    day: '2-digit',
    hour: '2-digit',
    minute: '2-digit',
    second: '2-digit',
  }).formatToParts(at)
  const get = (t: string) => Number(parts.find((p) => p.type === t)?.value)
  const asUtc = Date.UTC(get('year'), get('month') - 1, get('day'), get('hour'), get('minute'), get('second'))
  return Math.round((asUtc - Math.floor(at.getTime() / 1000) * 1000) / 60000)
}
export function gmtLabel(tz: string, at = new Date()): string {
  const m = zoneOffset(tz, at)
  const sign = m >= 0 ? '+' : '-'
  const h = Math.floor(Math.abs(m) / 60)
  const mm = Math.abs(m) % 60
  return `GMT${sign}${h}${mm ? ':' + String(mm).padStart(2, '0') : ''}`
}

/** Selectable zones; the system zone is added when missing. */
export const ZONES: Array<[string, string, string]> = [
  ['Pacific/Honolulu', '檀香山', 'Honolulu'],
  ['America/Anchorage', '安克雷奇', 'Anchorage'],
  ['America/Los_Angeles', '洛杉矶', 'Los Angeles'],
  ['America/Denver', '丹佛', 'Denver'],
  ['America/Chicago', '芝加哥', 'Chicago'],
  ['America/New_York', '纽约', 'New York'],
  ['America/Sao_Paulo', '圣保罗', 'São Paulo'],
  ['UTC', '协调世界时', 'UTC'],
  ['Europe/London', '伦敦', 'London'],
  ['Europe/Paris', '巴黎', 'Paris'],
  ['Europe/Berlin', '柏林', 'Berlin'],
  ['Europe/Moscow', '莫斯科', 'Moscow'],
  ['Asia/Dubai', '迪拜', 'Dubai'],
  ['Asia/Kolkata', '新德里', 'New Delhi'],
  ['Asia/Bangkok', '曼谷', 'Bangkok'],
  ['Asia/Singapore', '新加坡', 'Singapore'],
  ['Asia/Shanghai', '北京', 'Beijing'],
  ['Asia/Hong_Kong', '香港', 'Hong Kong'],
  ['Asia/Taipei', '台北', 'Taipei'],
  ['Asia/Tokyo', '东京', 'Tokyo'],
  ['Asia/Seoul', '首尔', 'Seoul'],
  ['Australia/Sydney', '悉尼', 'Sydney'],
  ['Pacific/Auckland', '奥克兰', 'Auckland'],
]
export function zoneName(tz: string): string {
  const known = ZONES.find((z) => z[0] === tz)
  if (known) return t(known[1], known[2])
  return tz.split('/').pop()?.replace(/_/g, ' ') || tz
}
export function zoneLabel(tz: string): string {
  return `(${gmtLabel(tz)}) ${zoneName(tz)}`
}
export function zoneOptions() {
  const list = ZONES.map((z) => z[0])
  const sys = systemZone()
  if (!list.includes(sys)) list.push(sys)
  return list
    .map((tz) => ({ value: tz, label: zoneLabel(tz), offset: zoneOffset(tz) }))
    .sort((a, b) => a.offset - b.offset)
}

type When = number | string | Date | null | undefined
function toDate(v: When): Date | null {
  if (v === null || v === undefined || v === '') return null
  const d = v instanceof Date ? v : new Date(v)
  return Number.isFinite(d.getTime()) ? d : null
}
function fmt(v: When, options: Intl.DateTimeFormatOptions, fallback = '—') {
  const d = toDate(v)
  if (!d) return fallback
  return new Intl.DateTimeFormat(isZh.value ? 'zh-CN' : 'en-US', {
    timeZone: zone.value,
    hourCycle: 'h23',
    ...options,
  }).format(d)
}
export const fmtDateTime = (v: When, fallback?: string) =>
  fmt(v, { year: 'numeric', month: '2-digit', day: '2-digit', hour: '2-digit', minute: '2-digit', second: '2-digit' }, fallback)
export const fmtShort = (v: When, fallback?: string) =>
  fmt(v, { month: '2-digit', day: '2-digit', hour: '2-digit', minute: '2-digit' }, fallback)
export const fmtDate = (v: When, fallback?: string) =>
  fmt(v, { year: 'numeric', month: '2-digit', day: '2-digit' }, fallback)
export const fmtTime = (v: When, fallback?: string) =>
  fmt(v, { hour: '2-digit', minute: '2-digit', second: '2-digit' }, fallback)
/** YYYY-MM-DD in the display zone. */
export function dayKey(v: When): string {
  const d = toDate(v)
  if (!d) return ''
  const parts = new Intl.DateTimeFormat('en-CA', { timeZone: zone.value, year: 'numeric', month: '2-digit', day: '2-digit' }).format(d)
  return parts
}

export function fmtNumber(n: unknown, digits = 2): string {
  const v = Number(n)
  if (n === null || n === undefined || !Number.isFinite(v)) return '—'
  return v.toLocaleString(isZh.value ? 'zh-CN' : 'en-US', { maximumFractionDigits: digits })
}
export function duration(ms: unknown): string {
  const v = Number(ms)
  if (ms === null || ms === undefined || !Number.isFinite(v)) return '—'
  return v < 1000 ? `${v} ms` : `${(v / 1000).toFixed(2)} s`
}
/** Relative "in 3 h" / "2 d ago". */
export function relative(v: When, now = Date.now()): string {
  const d = toDate(v)
  if (!d) return '—'
  const diff = d.getTime() - now
  const abs = Math.abs(diff)
  const unit = abs < 3600e3 ? [Math.max(1, Math.round(abs / 60e3)), '分钟', 'min'] : abs < 86400e3 ? [Math.round(abs / 3600e3), '小时', 'h'] : [Math.round(abs / 86400e3), '天', 'd']
  return diff >= 0 ? t(`${unit[0]} ${unit[1]}后`, `in ${unit[0]} ${unit[2]}`) : t(`${unit[0]} ${unit[1]}前`, `${unit[0]} ${unit[2]} ago`)
}
