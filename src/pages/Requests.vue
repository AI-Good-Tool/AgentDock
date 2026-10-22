<script setup lang="ts">
import ProviderMark from '../components/ProviderMark.vue'
import { computed, ref, onMounted } from 'vue'
import { Modal } from '@arco-design/web-vue'
import { IconDelete, IconRefresh } from '@arco-design/web-vue/es/icon'
import PageHead from '../components/PageHead.vue'
import RequestDrawer from '../components/RequestDrawer.vue'
import { call, type RequestLog } from '../api'
import { t, fmtShort, duration } from '../i18n'
import { state, act, refreshLogs, providerLabel, regionLabel, PROVIDERS } from '../store'
import { outcomeName, protocolName, logError } from '../lib/request-log'
import { requestTokenUsage, tokenCount, requestTokenBreakdown } from '../lib/request-usage'

const provider = ref('all')
const region = ref('all')
const result = ref('all')
const showInternal = ref(false)
const query = ref('')
const selected = ref<RequestLog | null>(null)
onMounted(refreshLogs)

const rows = computed(() => {
  const q = query.value.trim().toLowerCase()
  const all = [...state.active.map((l) => ({ ...l, outcome: 'running' })), ...state.logs]
  return all.filter(
    (l) =>
      (showInternal.value || l.protocol !== 'internal') &&
      (provider.value === 'all' || l.provider === provider.value) &&
      (region.value === 'all' || l.region === region.value) &&
      (result.value === 'all' ||
        (result.value === 'success'
          ? l.outcome === 'completed'
          : result.value === 'running' || result.value === 'retried'
            ? l.outcome === result.value
            : !['completed', 'running', 'retried'].includes(l.outcome))) &&
      (!q ||
        `${l.model} ${l.requestedModel} ${l.realModel} ${l.accountName} ${l.protocol} ${l.provider}`
          .toLowerCase()
          .includes(q))
  )
})
const visible = computed(() =>
  state.logs.filter((l) => showInternal.value || l.protocol !== 'internal')
)
const success = computed(() => visible.value.filter((l) => l.outcome === 'completed').length)
function clear() {
  Modal.confirm({
    title: t('清空请求记录', 'Clear request log'),
    content: t(
      '清空后不可恢复；Token 统计使用独立的用量账本，不受影响。',
      'This cannot be undone; token statistics use a separate ledger and are kept.'
    ),
    onOk: () => act('clear-logs', () => call('clear_logs'), t('已清空', 'Cleared'), [refreshLogs]),
  })
}
function rowClass(record: any) {
  return record.outcome === 'running' ? 'row-running' : ''
}
</script>

<template>
  <PageHead
    eyebrow="REQUEST ACTIVITY"
    :title="t('请求记录', 'Requests')"
    :desc="
      t(
        '查看每次请求使用的端、地区、账号、别名与真实模型、耗时和用量。',
        'See the provider, region, account, alias and real model, latency and usage of every request.'
      )
    "
  >
    <a-button @click="refreshLogs"
      ><template #icon><IconRefresh /></template>{{ t('刷新', 'Refresh') }}</a-button
    >
    <a-button status="danger" @click="clear"
      ><template #icon><IconDelete /></template>{{ t('清空记录', 'Clear') }}</a-button
    >
  </PageHead>
  <div class="metric-row">
    <div class="metric">
      <span>{{ t('已记录请求', 'Recorded') }}</span
      ><strong>{{ visible.length }}</strong>
      <p>{{ t('最近 1,000 条，重启后保留', 'Latest 1,000, kept across restarts') }}</p>
    </div>
    <div class="metric">
      <span>{{ t('成功率', 'Success rate') }}</span
      ><strong>{{
        visible.length ? Math.round((success / visible.length) * 100) + '%' : '—'
      }}</strong>
      <p>{{ success }} {{ t('次成功', 'succeeded') }}</p>
    </div>
    <div class="metric">
      <span>{{ t('进行中', 'In progress') }}</span
      ><strong>{{ state.active.length }}</strong>
      <p>{{ t('正在流式返回的请求', 'Streaming now') }}</p>
    </div>
    <div v-for="p in PROVIDERS" :key="p.key" class="metric">
      <span><ProviderMark :provider="p.key" style="margin-right: 6px" />{{ p.label }}</span>
      <strong>{{ visible.filter((l) => l.provider === p.key).length }}</strong>
      <p>
        {{ visible.filter((l) => l.provider === p.key && l.outcome === 'completed').length }}
        {{ t('次成功', 'succeeded') }}
      </p>
    </div>
  </div>
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
    <a-radio-group v-model="result" type="button">
      <a-radio value="all">{{ t('全部结果', 'All') }}</a-radio>
      <a-radio value="success">{{ t('成功', 'Success') }}</a-radio>
      <a-radio value="failed">{{ t('失败', 'Failed') }}</a-radio>
      <a-radio value="retried">{{ t('已换号重试', 'Retried') }}</a-radio>
      <a-radio value="running">{{ t('进行中', 'Running') }}</a-radio>
    </a-radio-group>
    <a-checkbox v-model="showInternal">{{
      t('显示内部操作', 'Show internal operations')
    }}</a-checkbox>
    <span class="grow"></span>
    <span class="muted">{{ t(`显示 ${rows.length} 条`, `${rows.length} shown`) }}</span>
    <a-input-search
      v-model="query"
      :placeholder="t('搜索模型、账号或协议', 'Search model, account or protocol')"
      style="width: 240px"
      allow-clear
    />
  </div>
  <section class="box" style="padding: 6px 8px">
    <a-table
      class="request-table table-tight"
      :data="rows"
      row-key="id"
      :bordered="false"
      :pagination="{ pageSize: 15 }"
      :row-class="rowClass"
      :scroll="{ x: 1200 }"
      @row-click="(r: any) => (selected = r)"
    >
      <template #columns>
        <a-table-column :title="t('时间', 'Time')" :width="120"
          ><template #cell="{ record }">{{ fmtShort(record.time) }}</template></a-table-column
        >
        <a-table-column :title="t('模型别名 / 真实模型', 'Alias / real model')" :width="240">
          <template #cell="{ record }"
            ><div class="cell-2">
              {{ record.model || record.requestedModel || '—'
              }}<small class="mono">{{ record.realModel || '—' }}</small>
            </div></template
          >
        </a-table-column>
        <a-table-column :title="t('端 / 地区', 'Provider / region')" :width="160">
          <template #cell="{ record }">
            <div class="cell-2">
              <span v-if="record.provider" class="ptag"
                ><ProviderMark :provider="record.provider" />{{
                  providerLabel(record.provider)
                }}</span
              >
              <span v-else class="muted">{{ t('未分配', 'Unassigned') }}</span>
              <small>{{ regionLabel(record.region) }}</small>
            </div>
          </template>
        </a-table-column>
        <a-table-column :title="t('账号', 'Account')" :width="150"
          ><template #cell="{ record }">{{ record.accountName || '—' }}</template></a-table-column
        >
        <a-table-column :title="t('协议', 'Protocol')" :width="170"
          ><template #cell="{ record }"
            >{{ protocolName(record.protocol)
            }}<small v-if="record.stream" class="muted" style="white-space: nowrap">
              · SSE</small
            ></template
          ></a-table-column
        >
        <a-table-column :title="t('Token', 'Tokens')" :width="160">
          <template #cell="{ record }"
            ><div class="cell-2">
              {{ tokenCount(requestTokenUsage(record.usage ?? record.details?.usage).total)
              }}<small>{{ requestTokenBreakdown(record.usage ?? record.details?.usage) }}</small>
            </div></template
          >
        </a-table-column>
        <a-table-column :title="t('耗时', 'Duration')" :width="100"
          ><template #cell="{ record }">{{
            record.outcome === 'running' ? '…' : duration(record.durationMs)
          }}</template></a-table-column
        >
        <a-table-column :title="t('结果', 'Result')" :width="220">
          <template #cell="{ record }">
            <div class="cell-2">
              <span
                class="state-pill"
                :class="{
                  bad: !['completed', 'running', 'retried'].includes(record.outcome),
                  warn: record.outcome === 'running' || record.outcome === 'retried',
                }"
                >{{ record.status && record.outcome !== 'running' ? record.status + ' · ' : ''
                }}{{ outcomeName(record.outcome) }}</span
              >
              <small :title="logError(record)">{{ logError(record) }}</small>
            </div>
          </template>
        </a-table-column>
      </template>
      <template #empty
        ><a-empty
          :description="
            t(
              '还没有请求记录。启动服务并连接客户端后，每次调用都会出现在这里。',
              'No requests yet. Start the gateway and connect a client.'
            )
          "
      /></template>
    </a-table>
  </section>
  <RequestDrawer :record="selected" @close="selected = null" />
</template>
<style>
.request-table .arco-table-tr {
  cursor: pointer;
}
.row-running .arco-table-td {
  background: var(--warn-soft) !important;
}
</style>
