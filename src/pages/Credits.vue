<script setup lang="ts">
import ProviderMark from '../components/ProviderMark.vue'
import { computed, ref, watch } from 'vue'
import { IconRefresh } from '@arco-design/web-vue/es/icon'
import PageHead from '../components/PageHead.vue'
import StatisticsPage from '../components/workbuddy/StatisticsPage.vue'
import RangePicker from '../components/RangePicker.vue'
import BarChart from '../components/BarChart.vue'
import { call } from '../api'
import { t, fmtNumber, fmtDateTime, fmtDate, zoneOffset } from '../i18n'
import { state, act, busy, refreshPool, isDisabled } from '../store'
import { inRange, daysOf, type RangeKey } from '../lib/range'

const tab = ref<'workbuddy' | 'qoder' | 'trae'>('workbuddy')

// ── Qoder: official daily usage (heatmap) + quota ──
const qstats = ref<any>(null)
const qError = ref('')
const qView = ref('all')
const qShareRange = ref<RangeKey>('today')
const qShareCustom = ref<[string, string] | null>(null)
const qChartRange = ref<RangeKey>('today')
const qChartCustom = ref<[string, string] | null>(null)
const qChartAcc = ref('all')
const qDetail = ref('')
const qUpdated = ref(0)
async function loadQoder() {
  qError.value = ''
  await act(
    'qstats',
    async () => {
      try {
        qstats.value = await call('qoder_credit_stats', { tzOffset: zoneOffset() })
        qUpdated.value = Date.now()
      } catch (e) {
        qError.value = String(e)
      }
    },
    undefined,
    []
  )
}
watch(tab, (v) => {
  if (v === 'qoder' && !qstats.value) loadQoder()
  if (v === 'trae') refreshPool()
})
const qAccounts = computed(() =>
  (qstats.value?.accounts ?? []).filter(
    (a: any) => qView.value === 'all' || a.edition === qView.value
  )
)
const heat = (a: any) =>
  (a?.heat ?? []).map((x: any) => ({ date: String(x.date || ''), value: Number(x.value) || 0 }))
function daily(accs: any[]) {
  const m = new Map<string, number>()
  for (const a of accs) for (const p of heat(a)) m.set(p.date, (m.get(p.date) || 0) + p.value)
  return [...m.entries()]
    .map(([date, value]) => ({ date, value }))
    .sort((a, b) => a.date.localeCompare(b.date))
}
const sum = (rows: any[], k: RangeKey, c?: [string, string] | null) =>
  rows.filter((p) => inRange(p.date, k, c)).reduce((s, p) => s + p.value, 0)
const qDaily = computed(() => daily(qAccounts.value))
const qMetrics = computed(() => {
  const total = qAccounts.value.reduce((s: number, a: any) => s + (Number(a.quotaTotal) || 0), 0)
  const used = qAccounts.value.reduce((s: number, a: any) => s + (Number(a.quotaUsed) || 0), 0)
  return {
    remaining: total > 0 ? Math.max(0, total - used) : null,
    today: sum(qDaily.value, 'today'),
    week: sum(qDaily.value, '7d'),
    month: sum(qDaily.value, 'month'),
  }
})
const qShare = computed(() => {
  const rows = qAccounts.value
    .filter((a: any) => a.ok)
    .map((a: any) => ({ a, usage: sum(heat(a), qShareRange.value, qShareCustom.value) }))
  const total = rows.reduce((s: number, r: any) => s + r.usage, 0)
  return { rows, total }
})
const qChart = computed(() => {
  const data =
    qChartAcc.value === 'all'
      ? qDaily.value
      : heat(qAccounts.value.find((a: any) => a.id === qChartAcc.value))
  const map = new Map(data.map((p: any) => [p.date, p.value]))
  const days = daysOf(
    qChartRange.value,
    qChartCustom.value,
    data.map((p: any) => p.date)
  )
  return days.map((d) => ({
    key: d,
    value: Number(map.get(d) ?? 0),
    tip: `${d} · ${fmtNumber(map.get(d) ?? 0)} ${t('积分', 'credits')}`,
  }))
})
const qDetailAcc = computed(
  () => qAccounts.value.find((a: any) => a.id === qDetail.value) ?? qAccounts.value[0]
)

// ── Trae: CN credits and international USD kept apart ──
const traeFilter = ref<'all' | 'cn' | 'intl'>('all')
const traeAccounts = computed(() =>
  (state.pool?.trae?.accounts ?? []).filter(
    (a: any) => traeFilter.value === 'all' || a.variant === traeFilter.value
  )
)
const traeCn = computed(() =>
  traeAccounts.value.filter((a: any) => a.variant === 'cn' && a.balance?.total != null)
)
const traeUsd = computed(() =>
  traeAccounts.value.filter((a: any) => a.balance?.unit === 'usd' && a.balance?.total != null)
)
const traeTotals = computed(() => ({
  total: traeCn.value.reduce((s: number, a: any) => s + a.balance.total, 0),
  used: traeCn.value.reduce((s: number, a: any) => s + a.balance.used, 0),
  remaining: traeCn.value.reduce((s: number, a: any) => s + a.balance.remaining, 0),
  usd: traeUsd.value.reduce((s: number, a: any) => s + a.balance.remaining, 0),
}))
async function traeRefreshAll() {
  await act(
    'trae-refresh-all',
    async () => {
      for (const a of traeAccounts.value.filter((a: any) => !isDisabled('trae', a.id)))
        await call('trae_refresh', { id: a.id }).catch(() => {})
    },
    t('用量已刷新', 'Usage refreshed')
  )
}
</script>

<template>
  <PageHead
    eyebrow="CREDITS & BILLING"
    :title="t('积分统计', 'Credits')"
    :desc="
      t(
        '三端分别保留官方口径；国内积分与国际美元额度不相加。',
        'Each provider keeps its official definition; China credits and international USD are never added together.'
      )
    "
  >
    <a-button v-if="tab === 'qoder'" type="primary" :loading="busy('qstats')" @click="loadQoder"
      ><template #icon><IconRefresh /></template>{{ t('刷新统计', 'Refresh') }}</a-button
    >
    <a-button
      v-if="tab === 'trae'"
      type="primary"
      :loading="busy('trae-refresh-all')"
      @click="traeRefreshAll"
      ><template #icon><IconRefresh /></template>{{ t('刷新用量', 'Refresh usage') }}</a-button
    >
  </PageHead>
  <a-radio-group v-model="tab" type="button" style="margin-bottom: 18px">
    <a-radio value="workbuddy"
      ><ProviderMark provider="workbuddy" style="margin-right: 6px" />WorkBuddy</a-radio
    >
    <a-radio value="qoder"
      ><ProviderMark provider="qoder" style="margin-right: 6px" />Qoder</a-radio
    >
    <a-radio value="trae"><ProviderMark provider="trae" style="margin-right: 6px" />Trae</a-radio>
  </a-radio-group>

  <StatisticsPage v-if="tab === 'workbuddy'" kind="credits" />

  <template v-if="tab === 'qoder'">
    <div class="filter-bar">
      <a-radio-group v-model="qView" type="button">
        <a-radio value="all">{{ t('全部', 'All') }}</a-radio>
        <a-radio value="cn">{{ t('国内版', 'China') }}</a-radio>
        <a-radio value="intl">{{ t('国际版', 'International') }}</a-radio>
      </a-radio-group>
      <span class="grow"></span>
      <span class="muted">{{
        qUpdated ? t('数据更新于 ', 'Updated ') + fmtDateTime(qUpdated) : ''
      }}</span>
    </div>
    <a-alert v-if="qError" type="error" style="margin-bottom: 14px">{{ qError }}</a-alert>
    <a-spin v-if="!qstats && busy('qstats')" style="width: 100%; margin: 40px 0" />
    <template v-if="qstats">
      <div class="metric-row">
        <div class="metric">
          <span>{{ t('剩余积分', 'Remaining') }}</span
          ><strong>{{ fmtNumber(qMetrics.remaining) }}</strong>
          <p>{{ t('套餐 + 资源包', 'Plan + add-ons') }}</p>
        </div>
        <div class="metric">
          <span>{{ t('今日消耗', 'Used today') }}</span
          ><strong>{{ fmtNumber(qMetrics.today) }}</strong>
        </div>
        <div class="metric">
          <span>{{ t('近 7 天消耗', 'Last 7 days') }}</span
          ><strong>{{ fmtNumber(qMetrics.week) }}</strong>
        </div>
        <div class="metric">
          <span>{{ t('本月消耗', 'This month') }}</span
          ><strong>{{ fmtNumber(qMetrics.month) }}</strong>
        </div>
      </div>
      <section class="box">
        <div class="box-head">
          <div>
            <h2>{{ t('账号消耗构成', 'Usage by account') }}</h2>
            <p>{{ t('各账号在所选范围内的积分消耗占比', 'Share of credit usage per account') }}</p>
          </div>
          <RangePicker v-model="qShareRange" v-model:custom="qShareCustom" />
        </div>
        <div class="kv">
          <span>{{ qShare.rows.length }} {{ t('个账号', 'accounts') }}</span
          ><b>{{ t('合计 ', 'Total ') }}{{ fmtNumber(qShare.total) }} {{ t('积分', 'credits') }}</b>
        </div>
        <div v-for="r in qShare.rows" :key="r.a.id" class="dist-row">
          <span
            >{{ r.a.name || r.a.id }}
            <small class="muted">{{
              r.a.edition === 'cn' ? t('国内', 'CN') : t('国际', 'Intl')
            }}</small></span
          >
          <div class="track">
            <i :style="{ width: `${qShare.total > 0 ? (r.usage / qShare.total) * 100 : 0}%` }"></i>
          </div>
          <b
            >{{ fmtNumber(r.usage) }} ·
            {{ qShare.total > 0 ? ((r.usage / qShare.total) * 100).toFixed(1) : 0 }}%</b
          >
        </div>
        <a-empty
          v-if="!qShare.total"
          :description="t('当前范围暂未观察到积分消耗', 'No usage in this range')"
        />
      </section>
      <section class="box">
        <div class="box-head">
          <div>
            <h2>{{ t('官方积分消耗', 'Official credit usage') }}</h2>
            <p>
              {{ t('来自 Qoder 官方每日消耗汇总', 'Daily summary from the official Qoder API') }}
            </p>
          </div>
          <div style="display: flex; gap: 8px">
            <a-select v-model="qChartAcc" style="width: 170px">
              <a-option value="all">{{ t('所有账号', 'All accounts') }}</a-option>
              <a-option v-for="a in qAccounts" :key="a.id" :value="a.id">{{
                a.name || a.id.slice(0, 12)
              }}</a-option>
            </a-select>
            <RangePicker v-model="qChartRange" v-model:custom="qChartCustom" />
          </div>
        </div>
        <BarChart :data="qChart" />
        <p class="hint">
          {{ t('当前口径合计 ', 'Total in range ')
          }}{{ fmtNumber(qChart.reduce((s, d) => s + d.value, 0)) }} {{ t('积分', 'credits') }}
        </p>
      </section>
      <section class="box">
        <div class="box-head">
          <div>
            <h2>{{ t('积分明细', 'Quota detail') }}</h2>
            <p>{{ t('套餐与加量包合并余量', 'Combined plan and add-on quota') }}</p>
          </div>
          <a-select
            :model-value="qDetailAcc?.id"
            style="width: 200px"
            @change="(v) => (qDetail = String(v))"
          >
            <a-option v-for="a in qAccounts" :key="a.id" :value="a.id">{{
              a.name || a.id.slice(0, 12)
            }}</a-option>
          </a-select>
        </div>
        <template v-if="qDetailAcc?.ok">
          <div class="kv">
            <span>{{ qDetailAcc.name }}</span
            ><b
              >{{ t('剩余 ', 'Remaining ')
              }}{{ fmtNumber(Math.max(0, qDetailAcc.quotaTotal - qDetailAcc.quotaUsed)) }} /
              {{ fmtNumber(qDetailAcc.quotaTotal) }}</b
            >
          </div>
          <div class="track">
            <i
              :style="{
                width: `${qDetailAcc.quotaTotal > 0 ? ((qDetailAcc.quotaTotal - qDetailAcc.quotaUsed) / qDetailAcc.quotaTotal) * 100 : 0}%`,
              }"
            ></i>
          </div>
          <p class="hint">
            {{ t('已用 ', 'Used ') }}{{ fmtNumber(qDetailAcc.quotaUsed) }} ·
            {{ t('累计消耗 ', 'total ') }}{{ fmtNumber(qDetailAcc.totalCredits) }} ·
            {{ t('单日峰值 ', 'daily peak ') }}{{ fmtNumber(qDetailAcc.peakCredits)
            }}{{ qDetailAcc.peakDate ? `（${qDetailAcc.peakDate}）` : '' }}
          </p>
        </template>
        <a-empty
          v-else
          :description="
            qDetailAcc
              ? t('该账号官方用量暂不可用', 'Official usage unavailable for this account')
              : t('暂无账号', 'No accounts')
          "
        />
      </section>
    </template>
  </template>

  <template v-if="tab === 'trae'">
    <div class="filter-bar">
      <a-radio-group v-model="traeFilter" type="button">
        <a-radio value="all">{{ t('全部', 'All') }}</a-radio>
        <a-radio value="cn">{{ t('国内版', 'China') }}</a-radio>
        <a-radio value="intl">{{ t('国际版', 'International') }}</a-radio>
      </a-radio-group>
    </div>
    <div class="metric-row">
      <div class="metric">
        <span>{{ t('国内版总额度', 'CN total') }}</span
        ><strong
          >{{ traeCn.length ? fmtNumber(traeTotals.total) : '—' }}<small>Credits</small></strong
        >
        <p>{{ t('官方 usage_summary', 'Official usage_summary') }}</p>
      </div>
      <div class="metric">
        <span>{{ t('国内版已消耗', 'CN used') }}</span
        ><strong
          >{{ traeCn.length ? fmtNumber(traeTotals.used) : '—' }}<small>Credits</small></strong
        >
        <p>{{ t('当前官方账期', 'Current billing period') }}</p>
      </div>
      <div class="metric">
        <span>{{ t('国内版剩余', 'CN remaining') }}</span
        ><strong
          >{{ traeCn.length ? fmtNumber(traeTotals.remaining) : '—' }}<small>Credits</small></strong
        >
      </div>
      <div class="metric">
        <span>{{ t('国际版剩余额度', 'Intl remaining') }}</span
        ><strong
          >{{ traeUsd.length ? '$' + fmtNumber(traeTotals.usd) : '—' }}<small>USD</small></strong
        >
        <p>{{ t('美元额度单独统计', 'USD tracked separately') }}</p>
      </div>
    </div>
    <section v-for="a in traeAccounts" :key="a.id" class="box">
      <div class="box-head">
        <h2>
          {{ a.note || a.username }}
          <span class="rtag">{{
            a.variant === 'cn' ? t('国内版', 'China') : t('国际版', 'International')
          }}</span>
        </h2>
        <small class="muted">{{
          a.snapshot ? fmtDateTime(a.snapshot.time) : t('待刷新', 'Not refreshed')
        }}</small>
      </div>
      <div class="kv">
        <span>{{ t('剩余额度', 'Remaining') }}</span
        ><b
          >{{ a.balance?.unit === 'usd' && a.balance?.remaining != null ? '$' : ''
          }}{{ fmtNumber(a.balance?.remaining) }}
          {{ a.balance?.unit === 'usd' ? 'USD' : t('积分', 'credits') }}</b
        >
      </div>
      <div class="kv">
        <span>{{ t('已使用 / 总额', 'Used / total') }}</span
        ><b>{{ fmtNumber(a.balance?.used) }} / {{ fmtNumber(a.balance?.total) }}</b>
      </div>
      <div
        v-for="(p, i) in a.snapshot?.usage?.user_entitlement_pack_list ?? []"
        :key="i"
        class="kv"
      >
        <span
          >{{ p.display_desc || p.group_name || t('额度包', 'Pack') }} ·
          {{
            p.expire_time
              ? t('有效期至 ', 'valid to ') + fmtDate(p.expire_time * 1000)
              : t('跟随官方账期', 'follows billing period')
          }}</span
        >
        <b
          >{{
            fmtNumber(
              a.balance?.unit === 'usd'
                ? p.entitlement_base_info?.quota?.basic_usage_limit
                : p.entitlement_base_info?.quota?.credits_limit
            )
          }}
          {{ a.balance?.unit === 'usd' ? 'USD' : t('积分', 'credits') }}</b
        >
      </div>
      <p v-if="a.snapshot?.errors?.usage" class="hint" style="color: var(--warn)">
        {{ a.snapshot.errors.usage }}
      </p>
    </section>
    <div v-if="!traeAccounts.length" class="empty-state">
      <strong>{{ t('暂无用量数据', 'No usage data') }}</strong
      >{{ t('先添加账号，再刷新获取积分和额度。', 'Add accounts, then refresh usage.') }}
    </div>
  </template>
</template>
