<script setup lang="ts">
import { computed } from 'vue'
import { IconCopy, IconInfoCircle } from '@arco-design/web-vue/es/icon'
import type { RequestLog } from '../api'
import { requestTokenUsage, tokenCount } from '../lib/request-usage'
import { outcomeName, protocolName, stageName, logTime, logError, logExplanation } from '../lib/request-log'
import { t, zone } from '../i18n'
import { copy, providerLabel, regionLabel, keyName } from '../store'
const props = defineProps<{ record: RequestLog | null }>()
const emit = defineEmits<{ close: [] }>()
const details = computed(() => props.record?.details)
const tokens = computed(() => requestTokenUsage(details.value?.usage ?? props.record?.usage))
const pretty = (value: unknown) => JSON.stringify(value ?? {}, null, 2)
const skipped = computed<Array<{ provider: string; region: string; reason: string; count: number }>>(() => details.value?.requestSummary?.skipped ?? [])
const strategyName: Record<string, [string, string]> = {
  RoundRobin: ['轮询', 'Round robin'],
  CreditFirst: ['积分优先', 'Credits first'],
  ExpiryFirst: ['临期优先', 'Expiring first'],
  Ordered: ['顺序优先', 'Ordered'],
  Random: ['随机', 'Random'],
}
const na = () => t('未提供', 'n/a')
const cloudCleanup = computed(() => {
  switch (details.value?.upstreamSummary?.cloudSessionCleanup) {
    case 'deleted': return t('已删除本次云端会话', 'This cloud session was deleted')
    case 'failed': return t('清理失败，详见运行日志', 'Cleanup failed; see runtime logs')
    case 'pending': return t('待清理；若请求已结束，将后台重试', 'Pending; retries in the background after the request ends')
    default: return ''
  }
})
</script>
<template>
  <a-drawer :visible="!!record" :width="780" :title="t('请求详情', 'Request details')" :footer="false" @cancel="emit('close')">
    <div v-if="record" class="request-detail">
      <div class="request-detail-title">
        <div>
          <h2>{{ record.model || record.requestedModel || '—' }}</h2>
          <small>{{ t('真实模型：', 'Upstream model: ') }}{{ record.modelName && record.modelName !== record.realModel ? record.modelName + ' · ' : '' }}{{ record.realModel || '—' }}</small>
          <span style="margin-left: 10px">{{ outcomeName(record.outcome) }}</span>
        </div>
        <a-button @click="copy(pretty(record))"><IconCopy /> {{ t('复制诊断信息', 'Copy diagnostics') }}</a-button>
      </div>
      <div class="request-explanation" :class="{ failed: record.status >= 400 }">
        <IconInfoCircle />
        <p>{{ logExplanation(record) }}</p>
      </div>
      <section>
        <h3>{{ t('请求与账号', 'Request and account') }}</h3>
        <a-descriptions :column="2" size="small" bordered>
          <a-descriptions-item :label="t('客户端传入模型', 'Requested model')">{{ record.requestedModel || '—' }}</a-descriptions-item>
          <a-descriptions-item :label="t('模型别名', 'Alias')">{{ record.model || '—' }}</a-descriptions-item>
          <a-descriptions-item :label="t('端', 'Provider')">{{ record.provider ? providerLabel(record.provider) : t('未分配', 'Unassigned') }}</a-descriptions-item>
          <a-descriptions-item :label="t('地区', 'Region')">{{ regionLabel(record.region) }}</a-descriptions-item>
          <a-descriptions-item :label="t('调用账号', 'Account')">{{ record.accountName || (record.accountId ? t('账号名称未采集', 'Name not captured') : t('未分配账号', 'Unassigned')) }}</a-descriptions-item>
          <a-descriptions-item :label="t('调度策略', 'Strategy')">{{ strategyName[record.strategy] ? t(...strategyName[record.strategy]) : record.strategy || '—' }}</a-descriptions-item>
          <a-descriptions-item :label="t('账号标识', 'Account ID')" :span="2"><span class="mono">{{ record.accountId || '—' }}</span></a-descriptions-item>
          <a-descriptions-item label="API Key">{{ keyName(record) || '—' }}</a-descriptions-item>
          <a-descriptions-item :label="t('Key 缩写', 'Key hint')"><span class="mono">{{ record.keyHint || '—' }}</span></a-descriptions-item>
          <a-descriptions-item v-if="details?.requestSummary?.omittedImages" :label="t('省略的历史图片', 'Omitted images')" :span="2">
            {{ details.requestSummary.omittedImages }} {{ t('张', '') }} · {{ (details.requestSummary.omittedImageBytes / 1048576).toFixed(1) }} MiB
          </a-descriptions-item>
          <a-descriptions-item v-if="skipped.length" :label="t('未参与本次调度', 'Skipped this time')" :span="2">
            <div v-for="(s, i) in skipped" :key="i">{{ providerLabel(s.provider) }} · {{ regionLabel(s.region) }} · {{ s.count }} {{ t('个账号', s.count === 1 ? 'account' : 'accounts') }}：{{ s.reason }}</div>
          </a-descriptions-item>
          <a-descriptions-item :label="t('开始时间', 'Started')">{{ details?.startedAt ? logTime(details.startedAt, true) : t('未采集', 'n/a') }}</a-descriptions-item>
          <a-descriptions-item :label="t('结束时间', 'Finished')">{{ logTime(record.time, true) }}</a-descriptions-item>
          <a-descriptions-item :label="t('总耗时', 'Duration')">{{ record.durationMs }} ms</a-descriptions-item>
          <a-descriptions-item :label="t('首字节耗时', 'First byte')">{{ (record.firstByteMs ?? details?.firstByteMs) == null ? t('未采集', 'n/a') : `${record.firstByteMs ?? details.firstByteMs} ms` }}</a-descriptions-item>
          <a-descriptions-item :label="t('客户端协议', 'Protocol')">{{ protocolName(record.protocol || details?.protocol) }}</a-descriptions-item>
          <a-descriptions-item :label="t('HTTP 状态', 'HTTP status')">{{ record.status }}</a-descriptions-item>
          <a-descriptions-item :label="t('请求路径', 'Path')" :span="2"><span class="mono">{{ details?.method || '—' }} {{ record.path || details?.clientPath || '—' }}</span></a-descriptions-item>
          <a-descriptions-item label="Request ID" :span="2"><span class="mono">{{ record.id }}</span></a-descriptions-item>
          <a-descriptions-item v-if="(record.attempt ?? 0) > 1 || record.outcome === 'retried'" :label="t('账号尝试', 'Attempt')">{{ t(`第 ${record.attempt} 个账号`, `Account #${record.attempt}`) }}</a-descriptions-item>
          <a-descriptions-item v-if="(record.attempt ?? 0) > 1 || record.outcome === 'retried'" :label="t('请求组', 'Request group')"><span class="mono">{{ record.requestGroup || record.id }}</span></a-descriptions-item>
        </a-descriptions>
      </section>
      <section>
        <h3>{{ t('上游响应', 'Upstream response') }}</h3>
        <a-descriptions :column="2" size="small" bordered>
          <a-descriptions-item :label="t('上游地址', 'Upstream URL')" :span="2"><span class="mono">{{ details?.upstreamUrl || t('未发送 / 未采集', 'Not sent / n/a') }}</span></a-descriptions-item>
          <a-descriptions-item :label="t('上游 HTTP', 'Upstream HTTP')">{{ details?.upstreamStatus ?? na() }}</a-descriptions-item>
          <a-descriptions-item :label="t('响应类型', 'Content type')">{{ details?.contentType || na() }}</a-descriptions-item>
          <a-descriptions-item :label="t('失败阶段', 'Failure stage')">{{ record.status >= 400 ? stageName(details?.errorStage) : '—' }}</a-descriptions-item>
          <a-descriptions-item :label="t('上游业务码', 'Upstream code')">{{ details?.error?.code ?? record.upstreamCode ?? na() }}</a-descriptions-item>
          <a-descriptions-item :label="t('上游 Request ID', 'Upstream request ID')" :span="2"><span class="mono">{{ details?.upstreamRequestId || na() }}</span></a-descriptions-item>
          <a-descriptions-item label="Trace ID" :span="2"><span class="mono">{{ details?.traceId || na() }}</span></a-descriptions-item>
          <a-descriptions-item label="Retry-After">{{ details?.retryAfter ? `${details.retryAfter} s` : na() }}</a-descriptions-item>
          <a-descriptions-item :label="t('响应字节数', 'Response bytes')">{{ details?.responseBytes ?? na() }}</a-descriptions-item>
          <a-descriptions-item :label="t('结束原因', 'Finish reason')">{{ details?.finishReason || na() }}</a-descriptions-item>
          <a-descriptions-item :label="t('上游结束标记', 'End marker')">{{ details?.streamComplete ? t('已收到', 'Received') : t('未收到 / 不适用', 'Not received / n/a') }}</a-descriptions-item>
          <a-descriptions-item v-if="cloudCleanup" :label="t('云端会话清理', 'Cloud session cleanup')" :span="2">{{ cloudCleanup }}</a-descriptions-item>
        </a-descriptions>
      </section>
      <section v-if="record.status >= 400">
        <h3>{{ t('错误详情（脱敏）', 'Error details (redacted)') }}</h3>
        <p class="request-error-message">{{ logError(record) || t('此记录未采集错误详情。', 'No error details captured.') }}</p>
        <pre>{{ pretty(details?.error) }}</pre>
      </section>
      <section>
        <h3>{{ t('客户端请求摘要', 'Client request summary') }}</h3>
        <pre>{{ pretty(details?.requestSummary) }}</pre>
      </section>
      <section>
        <h3>{{ t('转换后的上游请求摘要', 'Upstream request summary') }}</h3>
        <pre>{{ pretty(details?.upstreamSummary) }}</pre>
      </section>
      <section v-if="tokens.total != null">
        <h3>{{ t('上游返回用量', 'Upstream usage') }}</h3>
        <a-descriptions :column="2" size="small" bordered>
          <a-descriptions-item :label="t('总 Token', 'Total')">{{ tokenCount(tokens.total) }}</a-descriptions-item>
          <a-descriptions-item :label="t('输入（含缓存读取）', 'Input (incl. cache reads)')">{{ tokenCount(tokens.input) }}</a-descriptions-item>
          <a-descriptions-item :label="t('输出（含思考）', 'Output (incl. thinking)')">{{ tokenCount(tokens.output) }}</a-descriptions-item>
          <a-descriptions-item :label="t('缓存读取', 'Cache read')">{{ tokenCount(tokens.cacheRead) }}</a-descriptions-item>
          <a-descriptions-item :label="t('缓存写入', 'Cache write')">{{ tokenCount(tokens.cacheWrite) }}</a-descriptions-item>
          <a-descriptions-item :label="t('其中思考', 'Thinking')">{{ tokenCount(tokens.thinking) }}</a-descriptions-item>
        </a-descriptions>
      </section>
      <p class="request-detail-note">
        {{ t(`时间按 ${zone} 显示。仅保留参数与错误摘要，不保存请求正文、生成内容或认证头。`, `Times shown in ${zone}. Only parameters and error summaries are kept; no prompts, outputs or auth headers.`) }}
      </p>
    </div>
  </a-drawer>
</template>
<style>
.request-detail section {
  margin-top: 26px;
}
.request-detail-title {
  display: flex;
  justify-content: space-between;
  align-items: center;
  gap: 12px;
}
.request-detail-title h2 {
  margin: 0 0 6px;
  font-size: 22px;
}
.request-detail-title span {
  color: var(--muted);
}
.request-detail h3 {
  font-size: 14px;
  margin: 0 0 12px;
}
.request-explanation {
  display: flex;
  gap: 10px;
  padding: 14px;
  margin-top: 20px;
  background: var(--accent-soft);
  border-radius: 10px;
  color: var(--text-2);
}
.request-explanation.failed {
  background: var(--warn-soft);
  color: var(--warn);
}
.request-explanation svg {
  flex: none;
  margin-top: 3px;
}
.request-explanation p {
  margin: 0;
  line-height: 1.7;
}
.request-detail .arco-descriptions-value {
  word-break: break-word;
}
.request-detail pre {
  white-space: pre-wrap;
  overflow-wrap: anywhere;
  max-height: 350px;
  overflow: auto;
  padding: 16px;
  background: var(--surface-2);
  border: 1px solid var(--line);
  border-radius: 10px;
  font: 12px/1.7 ui-monospace, SFMono-Regular, Menlo, monospace;
}
.request-detail-note {
  font-size: 12px;
  line-height: 1.8;
  color: var(--muted);
  margin: 24px 0;
}
.request-error-message {
  white-space: pre-wrap;
  line-height: 1.7;
  color: var(--warn);
}
</style>
