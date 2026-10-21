import { t, isZh } from '../i18n'
export interface RequestTokenUsage {
  input: number | null
  output: number | null
  cacheRead: number | null
  cacheWrite: number | null
  thinking: number | null
  total: number | null
}
type Usage = Record<string, unknown> | null | undefined
const count = (v: unknown): number | null =>
  typeof v === 'number' && Number.isSafeInteger(v) && v >= 0 ? v : null
const nested = (v: unknown, field: string) =>
  v && typeof v === 'object' ? count((v as Record<string, unknown>)[field]) : null
export function requestTokenUsage(usage: Usage): RequestTokenUsage {
  const u = usage ?? {}
  const cacheRead =
    nested(u.prompt_tokens_details, 'cached_tokens') ?? count(u.cache_read_input_tokens)
  const cacheWrite = count(u.cache_creation_input_tokens) ?? count(u.prompt_cache_write_tokens)
  const nativeInput = count(u.input_tokens)
  const input =
    count(u.prompt_tokens) ?? (nativeInput == null ? null : nativeInput + (cacheRead ?? 0))
  const output = count(u.completion_tokens) ?? count(u.output_tokens)
  return {
    input,
    output,
    cacheRead,
    cacheWrite,
    thinking: nested(u.completion_tokens_details, 'reasoning_tokens'),
    // Cached reads are part of input; cache creation is separate, thinking is part of output.
    total:
      input != null && output != null ? input + output + (cacheWrite ?? 0) : count(u.total_tokens),
  }
}
export const tokenCount = (v: number | null | undefined) =>
  v == null ? '—' : new Intl.NumberFormat(isZh.value ? 'zh-CN' : 'en-US').format(v)
export function requestTokenBreakdown(usage: Usage) {
  const v = requestTokenUsage(usage)
  if (v.total == null) return t('上游未返回 Token 用量', 'No usage returned')
  const parts = [`${t('入', 'in')} ${tokenCount(v.input)}`, `${t('出', 'out')} ${tokenCount(v.output)}`]
  if (v.cacheWrite) parts.push(`${t('缓存写入', 'cache write')} ${tokenCount(v.cacheWrite)}`)
  return parts.join(' / ')
}
