<script setup lang="ts">
import ProviderMark from '../components/ProviderMark.vue'
import { computed, ref, watch, onMounted, onUnmounted } from 'vue'
import { IconRefresh } from '@arco-design/web-vue/es/icon'
import PageHead from '../components/PageHead.vue'
import StatisticsPage from '../components/workbuddy/StatisticsPage.vue'
import RangePicker from '../components/RangePicker.vue'
import BarChart from '../components/BarChart.vue'
import { call } from '../api'
import { t, fmtShort, duration, zoneOffset, isZh } from '../i18n'
import { state, busy, act, providerLabel, regionLabel, PROVIDERS } from '../store'
import { rangeMillis, daysOf, inRange, type RangeKey } from '../lib/range'
import { protocolName, outcomeName } from '../lib/request-log'

const source = ref<'proxy' | 'workbuddy' | 'qoder'>('proxy')
// Number format preference: K/M, 万/亿 or full.
function tok(n: unknown) {
  const v = Number(n)
  if (n == null || !Number.isFinite(v)) return '—'
  const f = state.app?.settings.tokenFormat ?? 'km'
  if (f === 'full') return v.toLocaleString(isZh.value ? 'zh-CN' : 'en-US')
  if (f === 'cn')
    return v >= 1e8
      ? (v / 1e8).toFixed(2) + '亿'
      : v >= 1e4
        ? (v / 1e4).toFixed(1) + '万'
        : String(v)
  return v >= 1e9
    ? (v / 1e9).toFixed(2) + 'B'
    : v >= 1e6
      ? (v / 1e6).toFixed(2) + 'M'
      : v >= 1e3
        ? (v / 1e3).toFixed(1) + 'K'
        : String(v)
}

// ── Proxy (unified ledger) ──
const provider = ref('all')
const region = ref('all')
const range = ref<RangeKey>('30d')
const custom = ref<[string, string] | null>(null)
const dist = ref<'models' | 'accounts' | 'providers' | 'regions' | 'realModels'>('models')
const stats = ref<any>(null)
const page = ref(1)
async function loadProxy() {
  const { from, to } = rangeMillis(range.value, custom.value)
  stats.value = await call('proxy_statistics', {
    filter: { provider: provider.value, region: region.value, from, to },
    offsetMinutes: zoneOffset(),
  }).catch(() => stats.value)
}
watch([provider, region, range, custom], () => {
  page.value = 1
  loadProxy()
})
let timer: ReturnType<typeof setInterval>
onMounted(() => {
  loadProxy()
  timer = setInterval(() => source.value === 'proxy' && !document.hidden && loadProxy(), 5000)
})
onUnmounted(() => clearInterval(timer))
const s = computed(() => stats.value?.summary ?? {})
const chart = computed(() => {
  const map = new Map((stats.value?.daily ?? []).map((d: any) => [d.key, d.total ?? 0]))
  return daysOf(range.value, custom.value, [...map.keys()] as string[]).map((d) => ({
    key: d,
    value: Number(map.get(d) ?? 0),
    tip: `${d} · ${tok(map.get(d) ?? 0)} tokens`,
  }))
})
const distRows = computed(() => (stats.value?.[dist.value] ?? []).slice(0, 12))
const distMax = computed(() => Math.max(1, ...distRows.value.map((r: any) => r.total || 0)))
const distLabel = (key: string) => {
  if (dist.value === 'providers') return providerLabel(key)
  if (dist.value === 'regions') {
    const [p, r] = key.split(':')
    return `${providerLabel(p)} · ${regionLabel(r)}`
  }
  return key
}
const requests = computed(() => stats.value?.requests ?? [])

// ── Qoder local estimate ──
const qEdition = ref<'cn' | 'intl'>('cn')
const qRange = ref<RangeKey>('30d')
const qCustom = ref<[string, string] | null>(null)
const qDist = ref<'projects' | 'models'>('projects')
const qstats = ref<any>(null)
const qError = ref('')
async function loadQoder() {
  qError.value = ''
  await act(
    'qtokens',
    async () => {
      try {
        qstats.value = await call('qoder_token_stats', {
          edition: qEdition.value,
          tzOffset: zoneOffset(),
        })
      } catch (e) {
        qError.value = String(e)
        qstats.value = null
      }
    },
    undefined,
    []
  )
}
watch(source, (v) => v === 'qoder' && !qstats.value && loadQoder())
watch(qEdition, loadQoder)
const qDaily = computed(() =>
  (qstats.value?.daily ?? []).filter((d: any) => inRange(d.date, qRange.value, qCustom.value))
)
const qChart = computed(() => {
  const map = new Map((qstats.value?.daily ?? []).map((d: any) => [d.date, d.tokens ?? 0]))
  return daysOf(qRange.value, qCustom.value, [...map.keys()] as string[]).map((d) => ({
    key: d,
    value: Number(map.get(d) ?? 0),
    tip: `${d} · ${tok(map.get(d) ?? 0)}`,
  }))
})
const qTotals = computed(() =>
  qDaily.value.reduce(
    (a: any, d: any) => ({
      tokens: a.tokens + (d.tokens || 0),
      in: a.in + (d.in || 0),
      out: a.out + (d.out || 0),
      requests: a.requests + (d.requests || 0),
    }),
    { tokens: 0, in: 0, out: 0, requests: 0 }
  )
)
const qDistRows = computed(() => (qstats.value?.[qDist.value] ?? []).slice(0, 12))
const qRequests = computed(() =>
  (qstats.value?.requests ?? []).filter((r: any) =>
    inRange(String(r.ts || '').slice(0, 10), qRange.value, qCustom.value)
  )
)
</script>

<template>
  <PageHead
    eyebrow="TOKEN ANALYTICS"
    :title="t('Token 统计', 'Token Usage')"
    :desc="
      t(
        '反代实际用量覆盖三端全部请求；也可查看 WorkBuddy 本机客户端用量与 Qoder 本地会话估算。',
        'Proxy usage covers every provider; WorkBuddy local client usage and Qoder local estimates are also available.'
      )
    "
  >
    <a-button v-if="source === 'proxy'" @click="loadProxy"
      ><template #icon><IconRefresh /></template>{{ t('刷新', 'Refresh') }}</a-button
    >
    <a-button v-if="source === 'qoder'" :loading="busy('qtokens')" @click="loadQoder"
      ><template #icon><IconRefresh /></template>{{ t('刷新', 'Refresh') }}</a-button
    >
  </PageHead>
  <a-radio-group v-model="source" type="button" style="margin-bottom: 18px">
    <a-radio value="proxy">{{ t('反代实际（全部端）', 'Proxy usage (all providers)') }}</a-radio>
    <a-radio value="workbuddy"
      ><ProviderMark provider="workbuddy" style="margin-right: 6px" />{{
        t('WorkBuddy 本机客户端', 'WorkBuddy local clients')
      }}</a-radio
    >
    <a-radio value="qoder"
      ><ProviderMark provider="qoder" style="margin-right: 6px" />{{
        t('Qoder 本地估算', 'Qoder local estimate')
      }}</a-radio
    >
  </a-radio-group>

  <template v-if="source === 'proxy'">
    <div class="filter-bar">
      <a-select v-model="provider" style="width: 140px">
        <a-option value="all">{{ t('全部端', 'All providers') }}</a-option>
        <a-option v-for="p in PROVIDERS" :key="p.key" :value="p.key">{{ p.label }}</a-option>
      </a-select>
      <a-select v-model="region" style="width: 120px">
        <a-option value="all">{{ t('全部地区', 'All regions') }}</a-option>
        <a-option value="cn">{{ t('国内版', 'China') }}</a-option>
        <a-option value="intl">{{ t('国际版', 'International') }}</a-option>
      </a-select>
      <RangePicker v-model="range" v-model:custom="custom" />
    </div>
    <div class="metric-row">
      <div class="metric">
        <span>{{ t('总 Token', 'Total tokens') }}</span
        ><strong>{{ tok(s.total) }}</strong>
        <p>
          {{
            t(
              '输入 + 输出 + 缓存写入；缓存读取已含在输入内',
              'Input + output + cache writes; cache reads are part of input'
            )
          }}
        </p>
      </div>
      <div class="metric">
        <span>{{ t('输入', 'Input') }}</span
        ><strong>{{ tok(s.input) }}</strong>
        <p>
          {{ t('缓存命中 ', 'Cache hit ')
          }}{{ s.cacheHitRate != null ? (s.cacheHitRate * 100).toFixed(1) + '%' : '—' }}
        </p>
      </div>
      <div class="metric">
        <span>{{ t('输出', 'Output') }}</span
        ><strong>{{ tok(s.output) }}</strong>
        <p>{{ t('其中思考 ', 'Thinking ') }}{{ tok(s.thinking) }}</p>
      </div>
      <div class="metric">
        <span>{{ t('请求数', 'Requests') }}</span
        ><strong>{{ s.records ?? 0 }}</strong>
        <p>
          {{ s.success ?? 0 }} {{ t('次成功 · ', 'succeeded · ') }}{{ s.withUsage ?? 0 }}
          {{ t('次返回用量', 'with usage') }}
        </p>
      </div>
    </div>
    <section class="box">
      <div class="box-head">
        <div>
          <h2>{{ t('每日 Token 消耗', 'Daily tokens') }}</h2>
          <p>{{ t('按所选时区的自然日汇总', 'Grouped by day in your time zone') }}</p>
        </div>
      </div>
      <BarChart :data="chart" />
    </section>
    <section class="box">
      <div class="box-head">
        <h2>{{ t('消耗分布', 'Distribution') }}</h2>
        <a-radio-group v-model="dist" type="button" size="small">
          <a-radio value="models">{{ t('按别名', 'By alias') }}</a-radio>
          <a-radio value="realModels">{{ t('按真实模型', 'By real model') }}</a-radio>
          <a-radio value="providers">{{ t('按端', 'By provider') }}</a-radio>
          <a-radio value="regions">{{ t('按端/地区', 'By provider/region') }}</a-radio>
          <a-radio value="accounts">{{ t('按账号', 'By account') }}</a-radio>
        </a-radio-group>
      </div>
      <div v-for="r in distRows" :key="r.key" class="dist-row">
        <span :title="r.key">{{ distLabel(r.key) }}</span>
        <div class="track"><i :style="{ width: `${((r.total || 0) / distMax) * 100}%` }"></i></div>
        <b>{{ tok(r.total) }} · {{ r.records }} {{ t('次', 'req') }}</b>
      </div>
      <a-empty
        v-if="!distRows.length"
        :description="t('所选范围暂无反代用量', 'No proxy usage in range')"
      />
    </section>
    <section class="box" style="padding: 8px">
      <a-table
        :data="requests"
        :pagination="{ pageSize: 20, current: page }"
        :bordered="false"
        class="table-tight"
        row-key="id"
        @page-change="(p: number) => (page = p)"
      >
        <template #columns>
          <a-table-column :title="t('时间', 'Time')" :width="130"
            ><template #cell="{ record }"
              ><span style="white-space: nowrap">{{ fmtShort(record.timestamp) }}</span></template
            ></a-table-column
          >
          <a-table-column :title="t('模型', 'Model')"
            ><template #cell="{ record }"
              ><div class="cell-2">
                {{ record.model }}<small class="mono">{{ record.realModel }}</small>
              </div></template
            ></a-table-column
          >
          <a-table-column :title="t('端 / 账号', 'Provider / account')"
            ><template #cell="{ record }"
              ><div class="cell-2">
                {{ providerLabel(record.provider) }} · {{ regionLabel(record.region)
                }}<small>{{ record.accountName }}</small>
              </div></template
            ></a-table-column
          >
          <a-table-column :title="t('协议', 'Protocol')"
            ><template #cell="{ record }">{{
              protocolName(record.protocol)
            }}</template></a-table-column
          >
          <a-table-column :title="t('输入', 'Input')" align="right"
            ><template #cell="{ record }">{{
              record.usageKnown ? tok(record.input) : '—'
            }}</template></a-table-column
          >
          <a-table-column :title="t('输出', 'Output')" align="right"
            ><template #cell="{ record }">{{
              record.usageKnown ? tok(record.output) : '—'
            }}</template></a-table-column
          >
          <a-table-column :title="t('缓存读取', 'Cache read')" align="right"
            ><template #cell="{ record }">{{
              record.usageKnown ? tok(record.cacheRead) : '—'
            }}</template></a-table-column
          >
          <a-table-column :title="t('耗时', 'Duration')" align="right"
            ><template #cell="{ record }">{{
              duration(record.durationMs)
            }}</template></a-table-column
          >
          <a-table-column :title="t('结果', 'Result')"
            ><template #cell="{ record }">{{
              outcomeName(record.outcome || '')
            }}</template></a-table-column
          >
        </template>
      </a-table>
      <p class="hint" style="padding: 0 10px">
        {{
          t(
            '仅记录数值用量与运行元数据，不保存对话正文。上游未返回用量的请求显示「—」，合计只汇总已知部分。',
            'Only numeric usage and metadata are stored. Requests without upstream usage show “—”; totals include known values only.'
          )
        }}
      </p>
    </section>
  </template>

  <StatisticsPage v-if="source === 'workbuddy'" kind="tokens" />

  <template v-if="source === 'qoder'">
    <div class="filter-bar">
      <a-radio-group v-model="qEdition" type="button">
        <a-radio value="cn">{{ t('国内版', 'China') }}</a-radio>
        <a-radio value="intl">{{ t('国际版', 'International') }}</a-radio>
      </a-radio-group>
      <RangePicker v-model="qRange" v-model:custom="qCustom" />
    </div>
    <div class="notice" style="margin-bottom: 14px">
      {{
        t(
          'Qoder 不公开逐请求真实 token 数，本地会话中 usage 恒为 0；此处为本地会话请求观测（main.sqlite assistant 消息）的估算口径，与反代实际用量分开统计。',
          'Qoder does not expose per-request token counts (local usage is always 0). This is an estimate from local session messages, kept separate from proxy usage.'
        )
      }}
    </div>
    <a-alert v-if="qError" type="error" style="margin-bottom: 14px">{{ qError }}</a-alert>
    <template v-if="qstats">
      <div class="metric-row">
        <div class="metric">
          <span>{{ t('估算 Token', 'Estimated tokens') }}</span
          ><strong>{{ tok(qTotals.tokens) }}</strong>
        </div>
        <div class="metric">
          <span>{{ t('输入', 'Input') }}</span
          ><strong>{{ tok(qTotals.in) }}</strong>
        </div>
        <div class="metric">
          <span>{{ t('输出', 'Output') }}</span
          ><strong>{{ tok(qTotals.out) }}</strong>
        </div>
        <div class="metric">
          <span>{{ t('请求数', 'Requests') }}</span
          ><strong>{{ qTotals.requests }}</strong>
        </div>
      </div>
      <section class="box">
        <div class="box-head">
          <h2>{{ t('每日 Token 消耗（估算）', 'Daily tokens (estimate)') }}</h2>
        </div>
        <BarChart :data="qChart" />
      </section>
      <section class="box">
        <div class="box-head">
          <h2>{{ t('消耗分布', 'Distribution') }}</h2>
          <a-radio-group v-model="qDist" type="button" size="small">
            <a-radio value="projects">{{ t('按项目', 'By project') }}</a-radio>
            <a-radio value="models">{{ t('按模型', 'By model') }}</a-radio>
          </a-radio-group>
        </div>
        <div v-for="r in qDistRows" :key="r.name" class="dist-row">
          <span :title="r.name">{{ r.name }}</span>
          <div class="track">
            <i
              :style="{
                width: `${((r.tokens || 0) / Math.max(1, ...qDistRows.map((x: any) => x.tokens || 0))) * 100}%`,
              }"
            ></i>
          </div>
          <b>{{ tok(r.tokens) }} · {{ r.requests }} {{ t('次', 'req') }}</b>
        </div>
      </section>
      <section class="box" style="padding: 8px">
        <a-table
          :data="qRequests"
          :pagination="{ pageSize: 50 }"
          :bordered="false"
          class="table-tight"
        >
          <template #columns>
            <a-table-column :title="t('时间', 'Time')" data-index="ts" :width="170" />
            <a-table-column :title="t('模型', 'Model')" data-index="model" />
            <a-table-column :title="t('项目', 'Project')"
              ><template #cell="{ record }"
                ><span class="mono" style="font-size: 12px">{{ record.cwd }}</span></template
              ></a-table-column
            >
            <a-table-column :title="t('会话', 'Session')" data-index="title" />
            <a-table-column :title="t('输入', 'Input')" align="right"
              ><template #cell="{ record }">{{ tok(record.inTokens) }}</template></a-table-column
            >
            <a-table-column :title="t('输出', 'Output')" align="right"
              ><template #cell="{ record }">{{ tok(record.outTokens) }}</template></a-table-column
            >
            <a-table-column :title="t('耗时', 'Duration')" align="right"
              ><template #cell="{ record }">{{
                duration(record.durationMs)
              }}</template></a-table-column
            >
          </template>
        </a-table>
      </section>
    </template>
  </template>
</template>
