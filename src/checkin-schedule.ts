import type { CheckinSchedule } from './api'
import { t } from './i18n'

export const defaultCheckinSchedule = (): CheckinSchedule => ({
  kind: 'daily', every: 1, time: '07:00', weekdays: [1, 2, 3, 4, 5], date: '',
})

export const checkinWeekdays = () => [
  ['周一', 'Mon'], ['周二', 'Tue'], ['周三', 'Wed'], ['周四', 'Thu'],
  ['周五', 'Fri'], ['周六', 'Sat'], ['周日', 'Sun'],
].map(([zh, en], index) => ({ value: index + 1, label: t(zh!, en!) }))

export function describeCheckinSchedule(schedule: CheckinSchedule): string {
  switch (schedule.kind) {
    case 'minutes': return t(`每 ${schedule.every} 分钟`, `Every ${schedule.every} minute(s)`)
    case 'hours': return t(`每 ${schedule.every} 小时`, `Every ${schedule.every} hour(s)`)
    case 'days': return t(`每 ${schedule.every} 天`, `Every ${schedule.every} day(s)`)
    case 'date': return schedule.date ? `${schedule.date} ${schedule.time} ${t('执行一次', '(once)')}` : t('选择日期后执行一次', 'Run once on the selected date')
    case 'weekly': {
      const days = checkinWeekdays().filter((day) => schedule.weekdays.includes(day.value)).map((day) => day.label)
      return `${t('每周', 'Weekly: ')} ${days.join(t('、', ', ')) || t('未选择星期', 'no days selected')} ${schedule.time}`
    }
    default: return `${t('每天', 'Daily at')} ${schedule.time}`
  }
}
