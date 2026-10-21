import { t } from '../i18n'
import type { Account } from '../wb-api'
import { dateKey } from './statistics.ts'
export type CheckinStatus = 'done' | 'pending' | 'inactive' | 'error' | 'unknown'
export const freshCheckin = (a: Account, now: number) =>
  !!a.checkinUpdatedAt && dateKey(a.checkinUpdatedAt) === dateKey(now)
export function checkinStatus(a: Account, now: number): CheckinStatus {
  if (a.checkinError && a.checkinErrorAt && dateKey(a.checkinErrorAt) === dateKey(now))
    return 'error'
  if (!freshCheckin(a, now)) return 'unknown'
  if (a.checkin?.today_checked_in) return 'done'
  return a.checkin?.active === false ? 'inactive' : 'pending'
}
export function windowIssue(start: string, end: string): string {
  if (!start && !end) return ''
  const minutes = (s: string) => {
    if (!/^([01]\d|2[0-3]):[0-5]\d$/.test(s) && s !== '24:00') return NaN
    const [h, m] = s.split(':').map(Number)
    return h! * 60 + m!
  }
  return minutes(start) < minutes(end)
    ? ''
    : t('结束时间需晚于开始时间（HH:MM，不支持跨午夜）；或同时留空', 'End must be after start (HH:MM, no midnight crossing), or leave both empty')
}
