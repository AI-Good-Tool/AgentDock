import type { RequestLog } from '../api'
import { t, fmtDateTime, fmtTime } from '../i18n'
import { providerLabel } from '../store'

const outcomes: Record<string, [string, string]> = {
  completed: ['成功', 'Success'],
  running: ['进行中', 'Running'],
  retried: ['已换号重试', 'Retried'],
  agent_disabled: ['辅助 Agent 已禁用', 'Auxiliary agent disabled'],
  upstream_rejected: ['上游拒绝', 'Upstream rejected'],
  upstream_unreachable: ['上游连接失败', 'Upstream unreachable'],
  upstream_http_error: ['上游 HTTP 错误', 'Upstream HTTP error'],
  upstream_protocol_error: ['上游协议异常', 'Upstream protocol error'],
  runtime_unavailable: ['本机协议运行时不可用', 'Local runtime unavailable'],
  client_unavailable: ['客户端配置不可用', 'Client config unavailable'],
  transport_error: ['上游连接失败', 'Upstream connection failed'],
  official_transport_unavailable: ['官方传输暂不可用', 'Official transport unavailable'],
  invalid_request: ['请求格式错误', 'Invalid request'],
  request_too_large: ['请求体过大', 'Request too large'],
  model_not_found: ['模型不存在', 'Model not found'],
  model_not_available: ['所选端/地区无此模型', 'Model unavailable in scope'],
  model_not_exposed: ['模型未开放', 'Model not exposed'],
  no_available_account: ['没有可用账号', 'No available account'],
  credential_unavailable: ['凭据不可用', 'Credential unavailable'],
  client_disconnected: ['客户端已断开', 'Client disconnected'],
  cancelled: ['请求已取消', 'Cancelled'],
  failed: ['失败', 'Failed'],
  sse_error: ['上游流内错误', 'In-stream error'],
  stream_error: ['流传输失败', 'Stream failed'],
  incomplete_stream: ['响应未完整结束', 'Incomplete stream'],
  aggregation_failed: ['响应聚合失败', 'Aggregation failed'],
  invalid_upstream_response: ['上游响应格式异常', 'Invalid upstream response'],
  unexpected_content_type: ['上游响应格式异常', 'Unexpected content type'],
  credit_query_failed: ['积分查询失败', 'Credit query failed'],
  encoding_failed: ['请求压缩失败', 'Encoding failed'],
  rejected: ['请求被拒绝', 'Rejected'],
}
export function outcomeName(value: string) {
  const o = outcomes[value]
  return o ? t(...o) : value
}
export function protocolName(value?: string) {
  const names: Record<string, string> = {
    'anthropic-messages': 'Anthropic Messages',
    'anthropic-count-tokens': 'Messages count_tokens',
    'zcode-messages': 'ZCode · Anthropic Messages',
    'zcode-chat': 'ZCode · Chat Completions',
    'zcode-responses': 'ZCode · Responses',
    'openai-chat': 'Chat Completions',
    'openai-responses': 'Responses',
  }
  if (value === 'internal') return t('内部操作', 'Internal')
  return names[value ?? ''] || t('内部操作 / 未采集', 'Internal / not captured')
}
const stages: Record<string, [string, string]> = {
  request_body: ['请求体读取', 'Request body'],
  conversion: ['协议转换', 'Protocol conversion'],
  agent_policy: ['Agent 请求策略', 'Agent policy'],
  encoding: ['请求压缩', 'Encoding'],
  validation: ['请求校验', 'Validation'],
  model: ['模型解析', 'Model resolution'],
  account_selection: ['账号分配', 'Account selection'],
  credentials: ['凭据读取', 'Credentials'],
  transport: ['连接上游', 'Transport'],
  runtime: ['本机协议运行时', 'Local runtime'],
  official_transport: ['本机官方运行时／设备 SDK', 'Official runtime / device SDK'],
  upstream: ['官方上游', 'Upstream'],
  stream: ['流传输', 'Streaming'],
  aggregation: ['非流式聚合', 'Aggregation'],
}
export function stageName(value?: string | null) {
  const s = stages[value ?? '']
  return s ? t(...s) : t('未提供', 'n/a')
}
export function logTime(value: string, full = false) {
  return full ? fmtDateTime(value) : fmtTime(value)
}
export function logError(record: RequestLog) {
  const error = record.details?.error
  if (String(error?.code) === '11128') return t('官方渠道校验拒绝（11128）', 'Official channel check rejected (11128)')
  return error?.displayMessage || error?.message || error?.msg || record.message || ''
}
export function logExplanation(record: RequestLog) {
  const p = providerLabel(record.provider)
  if (record.outcome === 'request_too_large')
    return t('请求体或转换后的请求超过本地大小限制，未发送到上游。请缩短会话历史或减少附件；不会静默截断上下文或自动重试。', 'The request exceeded the local size limit and was not sent. Shorten the history or attachments; context is never silently truncated or retried.')
  if (record.outcome === 'official_transport_unavailable')
    return t('请求在 WorkBuddy 本机官方传输或设备 SDK 阶段停止。请检查 WorkBuddy 安装版本、区域和设备令牌状态。', 'Stopped in the WorkBuddy local transport or device SDK. Check the installed version, region and device token.')
  if (String(record.details?.error?.code) === '11128')
    return t('官方业务码 11128：来自未获批准渠道的 API 调用。请求已分配账号并到达官方；此错误不表示号池未启用或没有余额。不会自动重试、换账号或换模型。', 'Official code 11128: API call from an unapproved channel. The request reached the official service; it is not a pool or balance problem. No automatic retry or account/model change.')
  if (record.outcome === 'retried')
    return t(`${p} 在开始生成前明确拒绝（401/403/429）或返回排队通知，本次尝试未向客户端输出任何内容，已自动换下一个可用账号重试（同一请求最多 3 个账号）。同组请求见请求组 ${record.requestGroup || record.id}。`, `${p} explicitly rejected (401/403/429) or queued the request before any output, so nothing reached the client and the next eligible account was tried (at most 3 per request). Group: ${record.requestGroup || record.id}.`)
  if (record.outcome === 'agent_disabled')
    return t('请求在本地 Agent 策略检查阶段停止，未向上游发送。可在接入设置中调整辅助 Agent 开关。', 'Stopped by the local agent policy before reaching the upstream. Adjust the auxiliary-agent switch in API Access.')
  if (record.outcome === 'upstream_rejected')
    return t(`请求已到达 ${p} 上游，并收到 HTTP ${record.details?.upstreamStatus ?? record.status} 拒绝。请查看业务码与上游提示；限流（429）或鉴权失败会让该账号暂时退出调度。`, `The request reached ${p} and was rejected with HTTP ${record.details?.upstreamStatus ?? record.status}. Check the business code; 429 or auth failures temporarily remove the account from scheduling.`)
  if (record.details?.errorStage === 'account_selection')
    return t('请求在本地账号分配阶段结束，尚未发送到上游。具体筛选原因见错误详情：端/地区范围、模型、启用、有效期、冷却、并发和模型限额都会参与筛选。', 'Ended during local account selection; nothing was sent. The reasons list scope, model, enablement, expiry, cooldown, concurrency and model limits.')
  if (['invalid_request', 'model_not_found', 'model_not_available', 'model_not_exposed'].includes(record.outcome))
    return t('请求在本地校验、模型解析或协议转换阶段失败，尚未分配账号，也未向上游发送模型请求。', 'Failed during validation, model resolution or conversion; no account was used.')
  if (record.outcome === 'completed') return t('请求正常完成。流式请求在收到上游结束标记后记为成功。', 'Completed. Streams count as successful after the upstream end marker.')
  return t('请结合失败阶段、错误详情和请求摘要定位。', 'Use the failure stage, error details and summaries to investigate.')
}
