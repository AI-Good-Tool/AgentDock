import type { Account, CreditResource } from '../wb-api'
import { t, fmtShort, fmtTime, isZh } from '../i18n'
export function accountName(a: Account) {
  if (a.displayField === 'note') return a.note || a.presentation?.nickname || a.name
  if (a.displayField === 'phone') return maskedPhone(a.presentation?.phoneNumber) || a.name
  if (a.displayField === 'nickname') return a.presentation?.nickname || a.name
  return a.presentation?.displayName || a.name
}
export function maskedPhone(phone?: string) {
  return phone?.replace(/^(\d{3})\d+(\d{4})$/, '$1****$2') || ''
}
/** Card phone line: masked unless the page-level reveal is on (keeps +86 / spacing intact). */
export function phoneText(phone: string | undefined | null, reveal: boolean) {
  const raw = String(phone || '').trim()
  if (!raw || reveal) return raw
  const digits = raw.replace(/\D/g, '')
  if (digits.length < 7) return raw.replace(/\d/g, '*')
  const local = digits.length > 11 && digits.startsWith('86') ? digits.slice(2) : digits
  return local.slice(0, 3) + '****' + local.slice(-4)
}
export function credits(value?: number) {
  return value === undefined || value === null
    ? '—'
    : new Intl.NumberFormat(isZh.value ? 'zh-CN' : 'en-US', { maximumFractionDigits: 2 }).format(value)
}
export function resourceRows(resources: CreditResource[] = [], now = Date.now()) {
  return resources
    .filter((r) => r.remaining > 0 && (r.expireAt == null || r.expireAt > now))
    .slice()
    .sort((a, b) => (a.expireAt ?? Infinity) - (b.expireAt ?? Infinity))
}
export function expiry(value: number | null | undefined) {
  return value ? fmtShort(value) : t('长期有效', 'No expiry')
}
export function remaining(reset: number, now: number) {
  const ms = reset - now
  if (ms <= 60000) return t('即将恢复', 'resets soon')
  const mins = Math.ceil(ms / 60000),
    hours = Math.floor(mins / 60)
  const span = (hours ? `${hours}h` : '') + (mins % 60 ? `${mins % 60}m` : '')
  return t(`${span} 后恢复`, `resets in ${span}`)
}
export function updated(value?: number) {
  return value ? fmtTime(value).slice(0, 5) : '—'
}
const names: Record<string, string> = {
  TCACA_code_001_PqouKr6QWV: 'CodeBuddy 个人体验版',
  TCACA_code_002_AkiJS3ZHF5: '版本基础用量',
  TCACA_code_003_FAnt7lcmRT: 'CodeBuddy 个人标准版',
  TCACA_code_005_maRGyrHhw1: '版本基础用量',
  TCACA_code_006_DbXS0lrypC: 'CodeBuddy 个人体验版',
  TCACA_code_007_nzdH5h4Nl0: '平台奖励积分',
  TCACA_code_008_cfWoLwvjU4: '版本基础用量',
  TCACA_code_009_0XmEQc2xOf: '购买积分',
  TCACA_code_023_4xbGhMrE6q: '版本基础用量',
  TCACA_code_026_BaESVICNoi: '版本基础用量',
  TCACA_code_027_0FCGVA6vSa: '版本基础用量',
  TCACA_code_028_NtpWi0jzXs: '版本赠送用量',
  TCACA_code_029_6wCGEWquYy: '平台奖励积分',
  TCACA_code_030_BjSt89qTvr: '平台奖励积分',
  TCACA_code_035_ArVxJcGDsm: '版本基础用量',
  TCACA_code_036_lupO5WgNdG: '购买积分',
  TCACA_code_037_WxOD3MpI2o: '版本赠送用量',
  TCACA_code_038_OhvqZtiPKr: '购买积分',
  TCACA_code_039_KRcQj7wUat: '版本基础用量',
  TCACA_code_040_mi9rCYg46x: '版本基础用量',
}
// Package-code labels ported from MIT WorkBuddy Switch / official client table.
export function resourceName(r: CreditResource) {
  return names[r.packageCode ?? ''] || r.packageName || r.packageCode || t('积分包', 'Credit pack')
}
