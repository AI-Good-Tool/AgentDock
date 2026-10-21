<script setup lang="ts">
import { t } from '../../i18n'
import { computed, onUnmounted, ref, watch } from 'vue'
import { Message, Modal } from '@arco-design/web-vue'
import { IconRefresh, IconInfoCircle } from '@arco-design/web-vue/es/icon'
import { call } from '../../wb-api'
import type {
  CreditStatistics,
  CreditStatsAccount,
  TokenStatistics,
  TokenStatsSource,
} from '../../statistics-types'
import {
  ranges,
  exact,
  sourceNames,
  emptyTotals,
  periodTokens,
  dateKey,
  requestDate,
  inRange,
  heatmapDays,
  type StatsRange,
} from '../../lib/statistics'
import { logTime } from '../../lib/request-log'
import StatsTrend from './StatsTrend.vue'
const props = defineProps<{ kind: 'credits' | 'tokens' }>()
const busy = ref(false),
  remoteBusy = ref(false),
  error = ref('')
const credit = ref<CreditStatistics | null>(null),
  token = ref<TokenStatistics | null>(null)
const range = ref<StatsRange>('30d'),
  variant = ref('all'),
  account = ref('all'),
  creditMode = ref('official')
const source = ref('proxy'),
  trendModel = ref('all'),
  day = ref(''),
  distribution = ref('models')
const selectedDetail = ref<Record<string, unknown> | null>(null),
  detailVisible = ref(false)
let generation = 0
let proxyBusy = false
async function refreshProxy() {
  if (
    props.kind !== 'tokens' ||
    source.value !== 'proxy' ||
    busy.value ||
    proxyBusy ||
    !token.value
  )
    return
  const current = generation
  proxyBusy = true
  try {
    const data = await call<TokenStatsSource>('wb_proxy_statistics')
    if (current !== generation || props.kind !== 'tokens' || !token.value) return
    token.value = {
      ...token.value,
      sources: [...token.value.sources.filter((s) => s.source !== 'proxy'), data],
    }
    error.value = ''
  } catch (e) {
    if (current === generation) error.value = String(e)
  } finally {
    proxyBusy = false
  }
}
const proxyTimer = setInterval(() => void refreshProxy(), 5000)
onUnmounted(() => {
  clearInterval(proxyTimer)
  generation++
})
async function load() {
  const current = ++generation,
    kind = props.kind
  busy.value = true
  error.value = ''
  try {
    if (kind === 'credits') {
      const data = await call<CreditStatistics>('wb_credit_statistics')
      if (current !== generation) return
      credit.value = data
      if (!['complete', 'partial'].includes(data.officialUsage?.status ?? ''))
        creditMode.value = 'observed'
    } else {
      const data = await call<TokenStatistics>('wb_token_statistics')
      if (current !== generation) return
      token.value = data
    }
  } catch (e) {
    if (current === generation) error.value = String(e)
  } finally {
    if (current === generation) busy.value = false
  }
}
watch(
  () => props.kind,
  () => {
    day.value = ''
    account.value = 'all'
    void load()
  },
  { immediate: true }
)
watch([range, variant, account, source, trendModel, creditMode], () => (day.value = ''))
watch(variant, () => (account.value = 'all'))
watch(source, () => {
  trendModel.value = 'all'
  void refreshProxy()
})
function refreshOfficial() {
  Modal.confirm({
    title: t('查询官方积分用量', 'Query official credit usage'),
    content:
      t('将使用当前已导入账号的现有登录凭据，向各自的 WorkBuddy 官方域名查询近 31 个自然日的积分请求明细，繁忙账号可能需要分页。不会发送模型对话、执行签到或刷新原账号凭据。此操作会访问生产接口，是否确认查询？', 'This uses the existing logins of imported accounts to query 31 days of credit request details from the official WorkBuddy domains (busy accounts may need several pages). No model requests, check-ins or token refreshes happen. It contacts production services — continue?'),
    okText: t('确认查询', 'Query'),
    cancelText: t('取消', 'Cancel'),
    onOk: async () => {
      remoteBusy.value = true
      try {
        credit.value = await call<CreditStatistics>('wb_refresh_official_statistics', {
          confirmed: true,
        })
        creditMode.value = 'official'
        Message.success(t('官方用量查询完成，请查看账号同步状态', 'Official usage queried; check each account’s sync status'))
      } catch (e) {
        Message.error(String(e))
      } finally {
        remoteBusy.value = false
      }
    },
  })
}
const official = computed(() => credit.value?.officialUsage)
const localAccounts = computed(() =>
  (credit.value?.accounts ?? []).filter(
    (a) => variant.value === 'all' || (a.variant ?? 'cn') === variant.value
  )
)
const accountOptions = computed(() => [
  { label: t('全部账号', 'All accounts'), value: 'all' },
  ...localAccounts.value.map((a) => ({ label: a.accountName, value: a.accountId })),
])
const visibleAccounts = computed(() =>
  localAccounts.value.filter((a) => account.value === 'all' || a.accountId === account.value)
)
const visibleIds = computed(() => new Set(visibleAccounts.value.map((a) => a.accountId)))
const officialRows = computed(() =>
  (official.value?.accounts ?? []).filter((a) => visibleIds.value.has(a.accountId))
)
const usingOfficial = computed(() => creditMode.value === 'official')
const creditDaily = computed(() => {
  const result = new Map<string, number>()
  const rows = usingOfficial.value ? officialRows.value.filter((a) => a.ok) : visibleAccounts.value
  for (const a of rows)
    for (const p of a.daily ?? [])
      if (inRange(p.date, range.value)) result.set(p.date, (result.get(p.date) ?? 0) + p.usage)
  return [...result]
    .sort(([a], [b]) => a.localeCompare(b))
    .map(([date, value]) => ({ date, value }))
})
const remaining = computed(() =>
  visibleAccounts.value
    .filter((a) => a.isCurrent)
    .reduce((sum, a) => sum + (a.currentRemaining ?? 0), 0)
)
const creditedAccounts = computed(() =>
  usingOfficial.value
    ? officialRows.value.filter((a) => a.ok).length
    : visibleAccounts.value.filter((a) => a.lastSnapshotAt != null).length
)
const usage = computed(() =>
  creditedAccounts.value ? creditDaily.value.reduce((s, p) => s + p.value, 0) : null
)
const latestObservation = computed(() =>
  Math.max(0, ...visibleAccounts.value.map((a) => a.lastSnapshotAt ?? 0))
)
const creditModels = computed(() => {
  if (!usingOfficial.value) return []
  const result = new Map<string, { model: string; credit: number; requestCount: number }>()
  for (const a of officialRows.value.filter((a) => a.ok))
    for (const p of a.daily ?? []) {
      if (!inRange(p.date, range.value) || (day.value && p.date !== day.value)) continue
      for (const m of p.models ?? []) {
        const old = result.get(m.model) ?? { model: m.model, credit: 0, requestCount: 0 }
        old.credit += m.credit
        old.requestCount += m.requestCount
        result.set(m.model, old)
      }
    }
  return [...result.values()].sort((a, b) => b.credit - a.credit)
})
const accountRows = computed(() =>
  visibleAccounts.value
    .map((a) => {
      const o = officialRows.value.find((o) => o.accountId === a.accountId)
      const daily = usingOfficial.value ? (o?.ok ? (o.daily ?? []) : []) : (a.daily ?? [])
      return {
        ...a,
        ok: usingOfficial.value ? o?.ok : a.lastSnapshotAt != null,
        error: usingOfficial.value ? o?.error : undefined,
        requests: o?.requestCount,
        usage:
          usingOfficial.value && !o?.ok
            ? null
            : daily.filter((d) => inRange(d.date, range.value)).reduce((s, d) => s + d.usage, 0),
      }
    })
    .sort((a, b) => (b.usage ?? -1) - (a.usage ?? -1))
)
const creditRequests = computed(() =>
  (official.value?.requests ?? []).filter(
    (r) =>
      visibleIds.value.has(r.accountId) &&
      inRange(requestDate(r.requestTime), range.value) &&
      (!day.value || requestDate(r.requestTime) === day.value)
  )
)
const localEvents = computed(() =>
  (credit.value?.events ?? []).filter(
    (e) =>
      (e.accountId ? visibleIds.value.has(e.accountId) : variant.value === 'all') &&
      inRange(e.date, range.value) &&
      (!day.value || e.date === day.value)
  )
)
const currentSource = computed(() => token.value?.sources.find((s) => s.source === source.value))
const tokenSummary = computed(() =>
  range.value === 'total'
    ? (currentSource.value?.summary ?? emptyTotals())
    : periodTokens(currentSource.value?.daily ?? [], range.value)
)
const trendPoints = computed(() => {
  const rows =
    trendModel.value === 'all'
      ? currentSource.value?.daily
      : currentSource.value?.dailyByModel?.[trendModel.value]
  return (rows ?? [])
    .filter((r) => inRange(r.key, range.value))
    .sort((a, b) => a.key.localeCompare(b.key))
    .map((r) => ({
      date: r.key,
      value: r.total,
      segments: [r.cacheRead, Math.max(0, r.input - r.cacheRead), r.output, r.cacheWrite],
      records: r.records,
    }))
})
const tokenModels = computed(() => {
  if (range.value === 'total') return currentSource.value?.models ?? []
  return Object.entries(currentSource.value?.dailyByModel ?? {})
    .map(([key, daily]) => ({ key, ...periodTokens(daily, range.value) }))
    .filter((r) => r.records > 0)
    .sort((a, b) => b.total - a.total)
})
const ranking = computed(() =>
  distribution.value === 'models' ? tokenModels.value : (currentSource.value?.projects ?? [])
)
const tokenRequests = computed(() =>
  (currentSource.value?.requests ?? []).filter(
    (r) =>
      inRange(dateKey(r.timestamp), range.value) &&
      (!day.value || dateKey(r.timestamp) === day.value) &&
      (trendModel.value === 'all' || r.model === trendModel.value)
  )
)
const heatmap = computed(() => heatmapDays(currentSource.value?.daily ?? []))
const heatMax = computed(() => Math.max(1, ...heatmap.value.map((p) => p.total)))
const heatColor = (n: number) =>
  n === 0
    ? '#edf2ef'
    : n / heatMax.value > 0.65
      ? '#299778'
      : n / heatMax.value > 0.3
        ? '#61bb96'
        : n / heatMax.value > 0.1
          ? '#9bd2b7'
          : '#c6e6d3'
function openDetail(record: unknown) {
  selectedDetail.value = record as Record<string, unknown>
  detailVisible.value = true
}
function selectAccount(record: unknown) {
  const id = (record as CreditStatsAccount).accountId
  if (id) account.value = id
}
const pct = (n: number | null | undefined) => (n == null ? '—' : `${(n * 100).toFixed(1)}%`)
const time = (ts: number | null | undefined) =>
  ts ? logTime(new Date(ts).toISOString(), true) : '—'
const requestTime = (value: string | number) => {
  const number = Number(value)
  const ts = Number.isFinite(number)
    ? number < 1e11
      ? number * 1000
      : number
    : Date.parse(
        /[zZ]|[+-]\d{2}:\d{2}$/.test(String(value))
          ? String(value)
          : String(value).replace(' ', 'T') + '+08:00'
      )
  return Number.isFinite(ts) ? time(ts) : String(value)
}
const detailFields = computed(() =>
  Object.entries(selectedDetail.value ?? {}).filter(
    ([key, value]) => !['title'].includes(key) && value != null && typeof value !== 'object'
  )
)
const labels = computed<Record<string, string>>(() => ({
  timestamp: t('请求时间', 'Request time'),
  requestTime: t('请求时间', 'Request time'),
  accountId: t('账号标识', 'Account ID'),
  accountName: t('调用账号', 'Account'),
  model: t('模型', 'Model'),
  requestId: 'Request ID',
  sessionId: t('会话标识', 'Session ID'),
  project: t('项目', 'Project'),
  credit: t('消耗积分', 'Credits'),
  input: t('输入 Token（含缓存命中）', 'Input tokens (incl. cache hits)'),
  output: t('输出 Token（含思考）', 'Output tokens (incl. thinking)'),
  cacheRead: t('缓存读取', 'Cache read'),
  cacheWrite: t('缓存写入', 'Cache write'),
  uncachedInput: t('新增输入', 'New input'),
  thinking: t('思考 Token（输出的子集）', 'Thinking tokens (part of output)'),
  total: t('总 Token', 'Total tokens'),
  records: t('请求数', 'Requests'),
  cacheHitRate: t('缓存命中率', 'Cache hit rate'),
  key: t('标识', 'Key'),
}))
</script>

<template>
  <div class="statistics-page">
    <div class="statistics-toolbar">
      <div class="statistics-filters">
        <template v-if="kind === 'credits'">
          <a-select
            v-model="variant"
            :options="[
              { label: t('全部地区', 'All regions'), value: 'all' },
              { label: t('国内版', 'China'), value: 'cn' },
              { label: t('国际版', 'International'), value: 'ai' },
            ]"
            style="width: 130px"
            :aria-label="t('统计地区', 'Region')"
          />
          <a-select
            v-model="account"
            :options="accountOptions"
            style="width: 190px"
            :aria-label="t('统计账号', 'Account')"
            allow-search
          />
          <a-select
            v-model="creditMode"
            :options="[
              { label: t('官方请求用量', 'Official usage'), value: 'official' },
              { label: t('本地观察消耗', 'Locally observed'), value: 'observed' },
            ]"
            style="width: 170px"
            :aria-label="t('积分统计口径', 'Definition')"
          />
        </template>
        <a-select
          v-else
          v-model="source"
          :options="Object.entries(sourceNames()).map(([value, label]) => ({ value, label }))"
          style="width: 200px"
          :aria-label="t('Token 数据来源', 'Token source')"
        />
        <a-select
          v-model="range"
          :options="ranges()"
          style="width: 130px"
          :aria-label="t('统计时间范围', 'Range')"
        />
      </div>
      <div class="statistics-filters">
        <a-button :loading="busy" @click="load"><IconRefresh /> {{ t('刷新本地统计', 'Refresh local stats') }}</a-button>
        <a-button
          v-if="kind === 'credits'"
          type="primary"
          :loading="remoteBusy"
          @click="refreshOfficial"
          >{{ t('查询官方用量', 'Query official usage') }}</a-button
        >
      </div>
    </div>
    <a-alert v-if="error" type="error" :show-icon="true">{{ error }}</a-alert>
    <div class="notice">
      <IconInfoCircle /><span v-if="kind === 'credits'"
        >{{
          usingOfficial
            ? t('官方请求用量来自 Switch 已有缓存或手动采集结果；不会与本地观察值相加。', 'Official usage comes from the Switch cache or a manual query and is never added to observed values.')
            : t('本地观察消耗由相邻余额快照下降推算，无法区分到期、消费与其他扣减；首次快照和余额增加只建立基线。', 'Observed usage is inferred from balance drops between snapshots and cannot tell expiry from spending; first snapshots and increases only set a baseline.')
        }}
        {{ t('签到独立记录，不计入消耗。日期按上海时间统计。', 'Check-ins are recorded separately. Days follow Shanghai time.') }}</span
      ><span v-else
        >{{ t('当前来源：', 'Source: ') }}{{ sourceNames()[source] }}. {{
          source === 'proxy'
            ? t('反代数值用量持久化保存，当前页面每 5 秒自动更新。', 'Proxy usage is persisted and refreshes every 5 seconds.')
            : t('此处读取对应客户端日志，与本地反代独立统计。', 'Read from the client logs, separate from the proxy.')
        }}{{ t('输入已含缓存读取，总量 = 输入 + 输出 + 缓存写入；思考包含在输出中。当前来源之间不相加，避免重复计数。', ' Input includes cache reads; total = input + output + cache writes; thinking is part of output. Sources are never added together.') }}</span
      >
    </div>

    <template v-if="kind === 'credits'">
      <div v-if="official?.status !== 'complete'" class="statistics-warning">
        {{ official?.status === 'partial' ? t('官方用量仅部分账号同步成功', 'Official usage synced for some accounts only') : t('官方用量尚不可用', 'Official usage not available yet') }}{{ t('，未同步账号显示“—”。默认读取现有缓存，点击“查询官方用量”并确认后才访问官方接口。', '; unsynced accounts show “—”. Cached data is read by default; official APIs are contacted only after you confirm “Query official usage”.') }}
      </div>
      <div class="statistics-metrics">
        <article>
          <span>{{ t('当前剩余积分', 'Remaining credits') }}</span><b>{{ exact(remaining) }}</b
          ><small>{{ t('仅当前账号 · 最近有效观察值', 'Current accounts · latest observation') }}</small>
        </article>
        <article>
          <span>{{ t('所选范围消耗', 'Used in range') }}</span><b>{{ exact(usage) }}</b
          ><small
            >{{ usingOfficial ? t('官方积分', 'Official') : t('本地观察积分', 'Observed') }} ·
            {{ t(`${creditedAccounts} 个账号有数据`, `${creditedAccounts} accounts with data`) }}</small
          >
        </article>
        <article>
          <span>{{ t('当前账号', 'Current accounts') }}</span><b>{{ visibleAccounts.filter((a) => a.isCurrent).length }}</b
          ><small>{{ t(`已选 ${visibleAccounts.length} 个账号（含历史）`, `${visibleAccounts.length} selected (incl. history)`) }}</small>
        </article>
        <article>
          <span>{{ usingOfficial ? t('官方最近采集', 'Last official query') : t('最近余额观察', 'Last observation') }}</span
          ><b class="statistics-date">{{
            time(usingOfficial ? official?.collectedAt : latestObservation)
          }}</b
          ><small>{{ t('采集窗口内的汇总，非实时余额', 'Summary of the collection window, not a live balance') }}</small>
        </article>
      </div>
      <section class="panel statistics-panel">
        <div class="statistics-heading">
          <h3>{{ t('每日积分消耗', 'Daily credit usage') }}</h3>
          <a-tag>{{ usingOfficial ? t('官方账单', 'Official billing') : t('本地观察', 'Observed') }}</a-tag>
        </div>
        <StatsTrend :points="creditDaily" @select="day = $event" />
      </section>
      <div class="statistics-two-columns">
        <section class="panel statistics-panel">
          <div class="statistics-heading">
            <h3>{{ t('账号消耗构成', 'Usage by account') }}</h3>
            <small>{{ t('所选范围', 'Selected range') }}</small>
          </div>
          <div
            v-for="a in accountRows"
            :key="a.accountId"
            class="statistics-rank"
            @click="account = a.accountId"
          >
            <span>{{ a.accountName }}</span
            ><b>{{ exact(a.usage) }}</b>
            <div>
              <i
                :style="{
                  width: `${a.usage && usage ? Math.min(100, (a.usage / usage) * 100) : 0}%`,
                }"
              />
            </div>
          </div>
          <a-empty v-if="!accountRows.length" />
        </section>
        <section class="panel statistics-panel">
          <div class="statistics-heading">
            <h3>{{ t('按模型分类', 'By model') }}</h3>
            <small>{{ usingOfficial ? t('官方用量 · 所选范围', 'Official · selected range') : t('本地快照不包含模型归因', 'Snapshots have no model attribution') }}</small>
          </div>
          <div v-for="m in creditModels.slice(0, 10)" :key="m.model" class="statistics-rank">
            <span>{{ m.model }}</span
            ><b>{{ exact(m.credit) }} {{ t('积分', 'credits') }}</b><small>{{ exact(m.requestCount) }} {{ t('次请求', 'requests') }}</small>
          </div>
          <a-empty
            v-if="!usingOfficial || !creditModels.length"
            :description="t('此口径或范围没有可用模型明细', 'No model details for this definition or range')"
          />
        </section>
      </div>
      <section class="panel statistics-panel">
        <div class="statistics-heading">
          <h3>{{ t('账号用量与余额', 'Accounts: usage and balance') }}</h3>
          <a-button v-if="account !== 'all'" type="text" @click="account = 'all'"
            >{{ t('恢复全部账号', 'All accounts') }}</a-button
          >
        </div>
        <a-table
          :data="accountRows"
          :pagination="{ pageSize: 10 }"
          :scroll="{ x: 1050 }"
          row-key="accountId"
          @row-click="selectAccount"
        >
          <template #columns>
            <a-table-column :title="t('账号', 'Account')" data-index="accountName" :width="150" />
            <a-table-column :title="t('地区', 'Region')" :width="90"
              ><template #cell="{ record }">{{
                record.variant === 'ai' ? t('国际版', 'International') : t('国内版', 'China')
              }}</template></a-table-column
            >
            <a-table-column :title="t('所选范围消耗', 'Used in range')" :width="150"
              ><template #cell="{ record }">{{ exact(record.usage) }}</template></a-table-column
            >
            <a-table-column :title="t('剩余积分', 'Remaining')" :width="130"
              ><template #cell="{ record }">{{
                exact(record.currentRemaining)
              }}</template></a-table-column
            >
            <a-table-column :title="t('最近余额观察', 'Last observation')" :width="180"
              ><template #cell="{ record }">{{
                time(record.lastSnapshotAt)
              }}</template></a-table-column
            >
            <a-table-column :title="t('今日签到', 'Check-in')" :width="100"
              ><template #cell="{ record }">{{
                record.checkedInToday
                  ? t('已签到', 'Done')
                  : record.checkedInToday === false
                    ? t('未签到', 'Not yet')
                    : t('未知', 'Unknown')
              }}</template></a-table-column
            >
            <a-table-column :title="t('同步状态', 'Sync')" :width="210"
              ><template #cell="{ record }"
                ><a-tag :color="record.ok ? 'green' : 'orange'">{{
                  record.ok ? t('有数据', 'Synced') : t('未同步', 'Not synced')
                }}</a-tag
                ><small>{{ record.error }}</small></template
              ></a-table-column
            >
          </template>
        </a-table>
      </section>
      <section class="panel statistics-panel">
        <div class="statistics-heading">
          <h3>{{ usingOfficial ? t('积分请求明细', 'Credit requests') : t('余额观察与签到事件', 'Observations and check-ins') }}</h3>
          <a-button v-if="day" type="text" @click="day = ''">{{ day }} · {{ t('清除日期筛选', 'Clear day filter') }}</a-button
          ><small v-else>{{ t('点击记录查看详情', 'Click a row for details') }}</small>
        </div>
        <a-table
          v-if="usingOfficial"
          :data="creditRequests"
          :pagination="{ pageSize: 15 }"
          :scroll="{ x: 900 }"
          @row-click="openDetail"
          ><template #columns>
            <a-table-column :title="t('请求时间', 'Request time')" :width="185"
              ><template #cell="{ record }">{{
                requestTime(record.requestTime)
              }}</template></a-table-column
            ><a-table-column :title="t('账号', 'Account')" data-index="accountName" :width="150" /><a-table-column
              :title="t('模型', 'Model')"
              data-index="model"
              :width="180"
            /><a-table-column :title="t('客户端', 'Client')" data-index="client" :width="140" /><a-table-column
              :title="t('积分', 'Credits')"
              :width="100"
              ><template #cell="{ record }">{{ exact(record.credit) }}</template></a-table-column
            ><a-table-column title="Request ID" data-index="requestId" :width="260" /> </template
        ></a-table>
        <a-table
          v-else
          :data="localEvents"
          :pagination="{ pageSize: 15 }"
          :scroll="{ x: 700 }"
          @row-click="openDetail"
          ><template #columns
            ><a-table-column :title="t('时间', 'Time')" :width="190"
              ><template #cell="{ record }">{{ time(record.ts) }}</template></a-table-column
            ><a-table-column :title="t('账号', 'Account')" data-index="accountName" :width="150" /><a-table-column
              :title="t('事件', 'Event')"
              :width="150"
              ><template #cell="{ record }">{{
                record.kind === 'usage' ? t('观察到余额下降', 'Balance drop observed') : t('签到记录', 'Check-in')
              }}</template></a-table-column
            ><a-table-column :title="t('数值 / 结果', 'Value / result')" :width="150"
              ><template #cell="{ record }">{{
                record.kind === 'usage' ? exact(record.amount) : record.result
              }}</template></a-table-column
            ></template
          ></a-table
        >
        <p class="statistics-note">
          {{ t('请求明细有条数上限，图表与汇总基于完整采集结果。失败账号不会按 0 消耗计算。本地快照保留 90 天，签到历史沿用 Switch 的留存范围。', 'Request details are capped; charts and totals use the full collection. Failed accounts are never counted as zero. Snapshots are kept 90 days.') }}
        </p>
      </section>
    </template>

    <template v-else>
      <div class="statistics-metrics token-metrics">
        <article>
          <span>{{ t('总 Token', 'Total tokens') }}</span><b>{{ exact(tokenSummary.total) }}</b
          ><small>{{ exact(tokenSummary.records) }} {{ t('次请求', 'requests') }}</small>
        </article>
        <article>
          <span>{{ t('输入 / 输出', 'Input / output') }}</span>
          <div class="token-pair">
            <div>
              <small>{{ t('输入', 'Input') }}</small><b>{{ exact(tokenSummary.input) }}</b>
            </div>
            <div>
              <small>{{ t('输出', 'Output') }}</small><b>{{ exact(tokenSummary.output) }}</b>
            </div>
          </div>
          <small>{{ t('输入含缓存读取，输出含思考', 'Input incl. cache reads; output incl. thinking') }}</small>
        </article>
        <article>
          <span>{{ t('缓存读取 / 写入', 'Cache read / write') }}</span>
          <div class="token-pair">
            <div>
              <small>{{ t('读取', 'Read') }}</small><b>{{ exact(tokenSummary.cacheRead) }}</b>
            </div>
            <div>
              <small>{{ t('写入', 'Write') }}</small><b>{{ exact(tokenSummary.cacheWrite) }}</b>
            </div>
          </div>
          <small>{{ t('命中率', 'Hit rate') }} {{ pct(tokenSummary.cacheHitRate) }}</small>
        </article>
        <article>
          <span>{{ t('日志扫描', 'Logs scanned') }}</span><b>{{ exact(currentSource?.filesScanned ?? 0) }}</b
          ><small>{{ exact(currentSource?.parseErrors ?? 0) }} {{ t('条解析异常 · 本地只读', 'parse errors · read-only') }}</small>
        </article>
      </div>
      <section class="panel statistics-panel">
        <div class="statistics-heading">
          <h3>{{ t('Token 趋势', 'Token trend') }}</h3>
          <a-select
            v-model="trendModel"
            :options="[
              { label: t('全部模型', 'All models'), value: 'all' },
              ...(currentSource?.models ?? []).map((m) => ({ label: m.key, value: m.key })),
            ]"
            style="width: 220px"
            :aria-label="t('趋势模型', 'Trend model')"
            allow-search
          />
        </div>
        <StatsTrend :points="trendPoints" token @select="day = $event" />
      </section>
      <section class="panel statistics-panel">
        <div class="statistics-heading">
          <h3>{{ t('年度活跃度', 'Yearly activity') }}</h3>
          <small>{{ t('当前来源 · 最近一年 · 悬停查看精确值', 'Current source · last year · hover for values') }}</small>
        </div>
        <div class="statistics-heatmap">
          <button
            v-for="p in heatmap"
            :key="p.day"
            :title="`${p.day} · ${exact(p.total)} Tokens`"
            :aria-label="`${p.day}: ${exact(p.total)} Tokens`"
            :style="{ background: heatColor(p.total) }"
            @click="day = p.day"
          />
        </div>
        <p class="statistics-note">
          {{
            source === 'proxy'
              ? t('反代数值用量来自 AgentDock 统一用量账本（WorkBuddy 部分）。', 'Proxy usage comes from the AgentDock usage ledger (WorkBuddy part).')
              : t('年度活跃度来自累计本地日志；与上方所选时间范围独立。', 'Yearly activity comes from cumulative local logs, independent of the selected range.')
          }}
        </p>
      </section>
      <div class="statistics-two-columns">
        <section class="panel statistics-panel">
          <div class="statistics-heading">
            <h3>{{ t('用量分布', 'Distribution') }}</h3>
            <a-radio-group
              v-model="distribution"
              type="button"
              size="small"
              :options="[
                { label: t('模型', 'Models'), value: 'models' },
                { label: source === 'proxy' ? t('账号', 'Accounts') : t('项目', 'Projects'), value: 'projects' },
              ]"
            />
          </div>
          <p class="statistics-note">
            {{
              distribution === 'models'
                ? t('模型排行按所选时间范围统计。', 'Model ranking follows the selected range.')
                : t('项目 / 账号排行为当前来源累计用量，独立于上方时间范围。', 'Project / account ranking is cumulative for this source.')
            }}
          </p>
          <div
            v-for="(r, i) in ranking.slice(0, 10)"
            :key="r.key"
            class="statistics-rank"
            @click="openDetail(r)"
          >
            <span>{{ i + 1 }} · {{ r.key }}</span
            ><b>{{ exact(r.total) }}</b>
            <div>
              <i
                :style="{ width: `${ranking[0]?.total ? (r.total / ranking[0].total) * 100 : 0}%` }"
              />
            </div>
          </div>
          <a-empty v-if="!ranking.length" />
        </section>
        <section class="panel statistics-panel">
          <div class="statistics-heading">
            <h3>{{ t('累计会话排行', 'Top sessions') }}</h3>
            <small>{{ t('当前来源 · 点击查看用量', 'Current source · click for usage') }}</small>
          </div>
          <div
            v-for="s in (currentSource?.sessions ?? []).slice(0, 10)"
            :key="s.key"
            class="statistics-rank"
            @click="openDetail(s)"
          >
            <span>{{ s.title || s.sessionId || s.key }}</span
            ><b>{{ exact(s.total) }}</b
            ><small>{{ s.project }} · {{ exact(s.records) }} {{ t('次', 'req') }}</small>
          </div>
          <a-empty v-if="!currentSource?.sessions?.length" :description="t('当前来源没有会话聚合', 'No session aggregation for this source')" />
        </section>
      </div>
      <section class="panel statistics-panel">
        <div class="statistics-heading">
          <h3>{{ t('请求明细', 'Requests') }} · {{ sourceNames()[source] }}</h3>
          <a-button v-if="day" type="text" @click="day = ''">{{ day }} · {{ t('清除日期筛选', 'Clear day filter') }}</a-button
          ><small
            >{{ t('已记录', 'Recorded') }} {{ exact(currentSource?.summary.records) }} · {{ t('点击查看缓存与思考用量', 'click for cache and thinking') }}</small
          >
        </div>
        <a-table
          :data="tokenRequests"
          :pagination="{ pageSize: 15 }"
          :scroll="{ x: 1200 }"
          @row-click="openDetail"
          ><template #columns>
            <a-table-column :title="t('时间', 'Time')" :width="180"
              ><template #cell="{ record }">{{ time(record.timestamp) }}</template></a-table-column
            ><a-table-column :title="t('模型', 'Model')" data-index="model" :width="180" /><a-table-column
              :title="source === 'proxy' ? t('账号', 'Account') : t('项目', 'Project')"
              data-index="project"
              :width="200"
            /><a-table-column :title="t('总 Token', 'Total')" :width="120"
              ><template #cell="{ record }"
                ><strong>{{ exact(record.total) }}</strong></template
              ></a-table-column
            ><a-table-column :title="t('输入', 'Input')" :width="110"
              ><template #cell="{ record }">{{ exact(record.input) }}</template></a-table-column
            ><a-table-column :title="t('输出', 'Output')" :width="100"
              ><template #cell="{ record }">{{ exact(record.output) }}</template></a-table-column
            ><a-table-column :title="t('缓存命中', 'Cache hit')" :width="110"
              ><template #cell="{ record }">{{ exact(record.cacheRead) }}</template></a-table-column
            ><a-table-column :title="t('缓存写入', 'Cache write')" :width="110"
              ><template #cell="{ record }">{{
                exact(record.cacheWrite)
              }}</template></a-table-column
            ><a-table-column :title="t('思考', 'Thinking')" :width="100"
              ><template #cell="{ record }">{{ exact(record.thinking) }}</template></a-table-column
            > </template
          ><template #empty
            ><a-empty
              :description="
                ['workbuddy', 'workbuddy-ai', 'codebuddy-ide'].includes(source)
                  ? t('当前客户端来源只提供聚合结果；切换到本地反代可查看反代请求明细', 'This client source only has aggregates; switch to the proxy source for request details')
                  : t('当前范围暂无请求明细；上游未返回用量的请求不会按 0 Token 统计', 'No requests in range; requests without upstream usage are not counted as 0')
              "
            />
            <a-button v-if="source !== 'proxy'" type="text" @click="source = 'proxy'"
              >{{ t('查看本地反代明细', 'View proxy details') }}</a-button
            ></template
          ></a-table
        >
        <p class="statistics-note">
          {{ t('最近最多展示 500 条数值明细；Switch 的 WorkBuddy / IDE 来源仅提供聚合结果。数据覆盖：', 'Up to 500 recent rows; WorkBuddy / IDE sources only provide aggregates. Coverage: ') }}{{
            time(currentSource?.coverageStartAt)
          }}
          — {{ time(currentSource?.coverageEndAt) }}。
        </p>
      </section>
    </template>
    <div v-if="busy" class="statistics-loading">{{ t('正在读取本地统计，请稍候…', 'Reading local statistics…') }}</div>
    <a-drawer v-model:visible="detailVisible" :title="t('统计明细', 'Details')" :width="620" :footer="false"
      ><a-descriptions :column="1" bordered
        ><a-descriptions-item
          v-for="[key, value] in detailFields"
          :key="key"
          :label="key === 'project' && source === 'proxy' ? t('调用账号', 'Account') : (labels[key] ?? key)"
          >{{
            key === 'timestamp'
              ? time(Number(value))
              : key === 'requestTime'
                ? requestTime(String(value))
                : key === 'cacheHitRate'
                  ? pct(Number(value))
                  : typeof value === 'number'
                    ? exact(value)
                    : value
          }}</a-descriptions-item
        ></a-descriptions
      >
      <p class="statistics-note">{{ t('仅包含统计标识和数值，不展示消息正文或认证信息。', 'Only identifiers and numbers; no message content or credentials.') }}</p></a-drawer
    >
  </div>
</template>

<style scoped>
.statistics-toolbar,
.statistics-filters,
.statistics-heading {
  display: flex;
  align-items: center;
  gap: 12px;
  flex-wrap: wrap;
}
.statistics-toolbar {
  justify-content: space-between;
  margin-bottom: 22px;
}
.statistics-heading {
  justify-content: space-between;
  margin-bottom: 22px;
}
.statistics-heading h3 {
  font-size: 16px;
  margin: 0;
}
.statistics-heading small,
.statistics-note {
  color: var(--c-8a9992);
  font-size: 12px;
  line-height: 1.8;
}
.statistics-metrics {
  display: grid;
  grid-template-columns: repeat(4, minmax(0, 1fr));
  gap: 18px;
  margin: 24px 0;
}
.statistics-metrics article {
  padding: 22px;
  background: var(--c-ffffff);
  border: 1px solid var(--c-e2e9e5);
  border-radius: 16px;
  display: flex;
  flex-direction: column;
  gap: 12px;
  min-width: 0;
}
.statistics-metrics span {
  color: var(--c-89998f);
  font-size: 12px;
}
.statistics-metrics b {
  font-size: 27px;
  font-weight: 600;
  letter-spacing: -0.7px;
  color: var(--c-344e41);
  overflow-wrap: anywhere;
}
.statistics-metrics small {
  color: var(--c-96a29b);
  font-size: 11px;
}
.statistics-metrics .statistics-date {
  font-size: 15px;
  line-height: 1.8;
  letter-spacing: 0;
}
.token-metrics {
  grid-template-columns: repeat(2, minmax(0, 1fr));
}
.token-metrics b {
  font-variant-numeric: tabular-nums;
}
.token-pair {
  display: grid;
  gap: 6px;
}
.token-pair > div {
  display: flex;
  flex-wrap: wrap;
  align-items: baseline;
  gap: 5px 10px;
}
.token-pair small {
  flex-shrink: 0;
}
.token-pair b {
  font-size: 23px;
}
@media (min-width: 1550px) {
  .token-metrics {
    grid-template-columns: repeat(4, minmax(0, 1fr));
  }
}
.statistics-panel {
  padding: 26px;
  margin-bottom: 22px;
  overflow: hidden;
}
.statistics-two-columns {
  display: grid;
  grid-template-columns: 1fr 1fr;
  gap: 22px;
}
.statistics-rank {
  display: grid;
  grid-template-columns: minmax(0, 1fr) auto;
  gap: 10px;
  padding: 14px 0;
  border-bottom: 1px solid var(--c-edf1ee);
  cursor: pointer;
}
.statistics-rank span {
  font-size: 13px;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.statistics-rank b {
  font-size: 13px;
  font-weight: 500;
  color: var(--c-466d59);
}
.statistics-rank > div {
  grid-column: 1/-1;
  height: 5px;
  border-radius: 6px;
  background: var(--c-f1f5f2);
}
.statistics-rank i {
  display: block;
  height: 5px;
  border-radius: 6px;
  background: var(--c-78c9a4);
}
.statistics-rank small {
  color: var(--c-9aa79f);
  font-size: 11px;
  grid-column: 1/-1;
  overflow-wrap: anywhere;
}
.statistics-warning {
  padding: 15px 18px;
  border: 1px solid var(--c-eddcca);
  background: var(--c-fff9f0);
  color: var(--c-a5804e);
  line-height: 1.8;
  font-size: 12px;
  border-radius: 10px;
  margin-top: 16px;
}
.statistics-heatmap {
  display: grid;
  grid-template-rows: repeat(7, 12px);
  grid-auto-flow: column;
  grid-auto-columns: 12px;
  gap: 4px;
  overflow-x: auto;
  padding-bottom: 10px;
}
.statistics-heatmap button {
  border: 0;
  border-radius: 3px;
  width: 12px;
  height: 12px;
  cursor: pointer;
}
.statistics-heatmap button:hover {
  outline: 2px solid var(--c-58977c);
}
.statistics-loading {
  position: fixed;
  right: 35px;
  bottom: 25px;
  background: var(--c-ffffff);
  padding: 15px 24px;
  border: 1px solid var(--c-dfe9e1);
  box-shadow: 0 5px 25px #294d3714;
  border-radius: 12px;
  color: var(--c-668574);
  font-size: 13px;
  z-index: 20;
}
.statistics-page :deep(.arco-table-tr) {
  cursor: pointer;
}
.statistics-page :deep(.arco-descriptions-value) {
  word-break: break-word;
}
.statistics-page :deep(.arco-table-td small) {
  display: block;
  color: var(--c-9ba8a0);
  font-size: 11px;
  margin-top: 5px;
}
@media (max-width: 1200px) {
  .statistics-metrics {
    grid-template-columns: repeat(2, minmax(0, 1fr));
  }
  .statistics-two-columns {
    grid-template-columns: 1fr;
  }
}
@media (max-width: 800px) {
  .statistics-panel {
    padding: 18px;
  }
  .statistics-toolbar {
    align-items: flex-start;
  }
}
</style>
