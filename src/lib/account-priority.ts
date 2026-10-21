import type { Account } from '../wb-api'

export type AccountPriority = 'roundRobin' | 'expiryFirst' | 'creditFirst'
export const priorityOptions: Array<{ value: AccountPriority; label: string }> = [
  { value: 'expiryFirst', label: '临期优先' },
  { value: 'creditFirst', label: '积分优先' },
  { value: 'roundRobin', label: '轮询' },
]

// Keep the same cached-resource rules as Rust account_priority::credit_rank.
export function creditRank(credit: Account['credit'], now: number) {
  if (!credit?.ok) return { expiry: Infinity, remaining: -Infinity }
  if (!Array.isArray(credit.resources)) {
    const n = credit.totalRemaining
    return {
      expiry: Infinity,
      remaining: typeof n === 'number' && Number.isFinite(n) && n >= 0 ? n : -Infinity,
    }
  }
  let expiry = Infinity,
    remaining = 0
  for (const r of credit.resources) {
    if (!Number.isFinite(r.remaining) || r.remaining <= 0) continue
    if (r.expireAt != null && r.expireAt <= now) continue
    remaining += r.remaining
    if (r.expireAt != null && Number.isFinite(r.expireAt)) expiry = Math.min(expiry, r.expireAt)
  }
  return { expiry, remaining: Number.isFinite(remaining) ? remaining : -Infinity }
}

export function compareAccounts(a: Account, b: Account, priority: AccountPriority, now: number) {
  if (priority === 'roundRobin') return 0
  const ar = creditRank(a.credit, now),
    br = creditRank(b.credit, now)
  const av = priority === 'expiryFirst' ? ar.expiry : -ar.remaining
  const bv = priority === 'expiryFirst' ? br.expiry : -br.remaining
  return av === bv ? 0 : av < bv ? -1 : 1
}
