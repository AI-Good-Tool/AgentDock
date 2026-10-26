<script setup lang="ts">
import { computed, ref } from 'vue'
import { Modal } from '@arco-design/web-vue'
import { IconDelete } from '@arco-design/web-vue/es/icon'
import ProviderMark from './ProviderMark.vue'
import { call, type ProviderKey } from '../api'
import type { Account, CheckinRecord } from '../wb-api'
import { t, fmtDateTime, fmtNumber } from '../i18n'
import { state, act, busy, providerLabel } from '../store'
import { accountName } from '../lib/account-display'

type Tone = 'ok' | 'warn' | 'bad' | 'off'
interface Row {
  key: string
  at: number
  provider: ProviderKey
  account: string
  trigger?: string
  action: string
  result: string
  tone: Tone
  credits?: number | null
  detail?: string
}

const view = ref<'all' | ProviderKey>('all')
const triggers: Record<string, [string, string]> = {
  manual: ['手动', 'Manual'],
  auto: ['自动', 'Auto'],
  startup: ['启动后自动', 'At launch'],
  scheduled: ['定时', 'Scheduled'],
  launchd: ['系统定时任务', 'System task'],
}
const triggerName = (v?: string) => (v && triggers[v] ? t(...triggers[v]) : '—')

function workbuddyRows(): Row[] {
  const wb = state.pool?.workbuddy
  const accounts: Account[] = wb?.accounts ?? []
  const since = state.now - 30 * 86400000
  return ((wb?.checkinRecords ?? []) as CheckinRecord[])
    .filter((r) => r.at >= since)
    .map((r, i) => {
      const a = accounts.find((x) => x.id === r.accountId)
      const checked = r.status?.today_checked_in === true
      const inactive = r.status?.active === false
      return {
        key: `workbuddy:${r.at}:${i}`,
        at: r.at,
        provider: 'workbuddy',
        account: a ? accountName(a) : t('已移除账号', 'Removed account'),
        trigger: r.trigger,
        action: r.submit ? t('签到', 'Check in') : t('查询状态', 'Query'),
        result: r.error
          ? t('失败', 'Failed')
          : checked
            ? t('今日已签到', 'Checked in')
            : inactive
              ? t('活动未开放', 'Not open')
              : t('待签到', 'Pending'),
        tone: r.error ? 'bad' : checked ? 'ok' : inactive ? 'off' : 'warn',
        credits: r.submit && checked ? r.status?.today_credit : null,
        detail: r.error ?? undefined,
      }
    })
}

const qoderPhases: Record<string, [string, string, Tone]> = {
  claimed: ['领取成功', 'Claimed', 'ok'],
  nothing: ['无可领', 'Nothing to claim', 'off'],
  dry_run: ['干跑', 'Dry run', 'off'],
  login_required: ['需重新登录', 'Login required', 'bad'],
  error: ['失败', 'Failed', 'bad'],
}
function qoderRows(): Row[] {
  return (state.pool?.qoder?.claimLog ?? []).flatMap((entry: any, i: number) => {
    // Written in the system's local time ("YYYY-MM-DD HH:MM:SS").
    const at = new Date(String(entry.time ?? '').replace(' ', 'T')).getTime() || 0
    return (entry.results ?? []).map((r: any, j: number): Row => {
      const [zh, en, tone] = qoderPhases[r.phase] ?? [r.phase ?? '—', r.phase ?? '—', 'off']
      return {
        key: `qoder:${i}:${j}`,
        at,
        provider: 'qoder',
        account: `${r.name ?? ''} · ${r.edition === 'cn' ? t('国内版', 'China') : t('国际版', 'International')}`,
        trigger: entry.trigger,
        action: t('领取积分', 'Claim credits'),
        result: t(zh, en),
        tone,
        credits: r.credits > 0 ? r.credits : null,
        detail: r.phase === 'claimed' ? undefined : r.msg,
      }
    })
  })
}

function traeRows(): Row[] {
  return (state.pool?.trae?.checkinHistory ?? []).map((r: any, i: number): Row => ({
    key: `trae:${i}`,
    at: Date.parse(r.time) || 0,
    provider: 'trae',
    account: r.accountName ?? '—',
    trigger: r.source,
    action: t('签到', 'Check in'),
    result:
      r.status === 'success' ? t('签到成功', 'Checked in') : r.status === 'already' ? t('已领取', 'Already claimed') : t('失败', 'Failed'),
    tone: r.status === 'success' ? 'ok' : r.status === 'already' ? 'off' : 'bad',
    credits: r.status === 'success' ? r.credits : null,
    detail: r.error,
  }))
}

const all = computed<Record<ProviderKey, Row[]>>(() => ({ workbuddy: workbuddyRows(), qoder: qoderRows(), trae: traeRows() }))
const rows = computed(() =>
  (view.value === 'all' ? [...all.value.workbuddy, ...all.value.qoder, ...all.value.trae] : all.value[view.value]).sort(
    (a, b) => b.at - a.at
  )
)
const views = computed(() => [
  { key: 'all' as const, label: t('全部', 'All'), count: all.value.workbuddy.length + all.value.qoder.length + all.value.trae.length },
  ...(['workbuddy', 'qoder', 'trae'] as ProviderKey[]).map((key) => ({ key, label: providerLabel(key), count: all.value[key].length })),
])

const scopeName = computed(() => (view.value === 'all' ? t('全部平台', 'all platforms') : providerLabel(view.value)))
function clearHistory() {
  const provider = view.value === 'all' ? null : view.value
  Modal.confirm({
    title: t(`清除${scopeName.value}的签到记录`, `Clear check-in history of ${scopeName.value}`),
    content: t(
      '只删除记录列表，不影响账号的今日签到状态与积分；清除后不可恢复。',
      "Only the history list is deleted; accounts' check-in status and credits are kept. This cannot be undone."
    ),
    okButtonProps: { status: 'danger' },
    onOk: () => act('checkin-clear', () => call('checkin_clear_history', { provider }), t('签到记录已清除', 'History cleared')),
  })
}
</script>
<template>
  <section class="box checkin-history-box">
    <div class="box-head">
      <div>
        <h2>{{ t('签到记录', 'Check-in history') }}</h2>
        <p>{{ t('三个平台的签到与领取记录，仅本机保存：WorkBuddy 最近 30 天，Qoder 最近 30 次领取，Trae 最近 300 条。', 'Check-ins and claims of all three platforms, stored locally: WorkBuddy 30 days, the last 30 Qoder claim runs, Trae 300 entries.') }}</p>
      </div>
      <a-button size="small" :disabled="!rows.length" :loading="busy('checkin-clear')" @click="clearHistory"
        ><template #icon><IconDelete /></template
        >{{ view === 'all' ? t('清除全部记录', 'Clear all') : t(`清除 ${scopeName} 记录`, `Clear ${scopeName}`) }}</a-button
      >
    </div>
    <a-radio-group v-model="view" type="button" class="history-views">
      <a-radio v-for="v in views" :key="v.key" :value="v.key"
        ><ProviderMark v-if="v.key !== 'all'" :provider="v.key" />{{ v.label }}<span class="history-count">{{ v.count }}</span></a-radio
      >
    </a-radio-group>
    <a-table
      :data="rows"
      row-key="key"
      :pagination="{ pageSize: 10, showTotal: true }"
      :scroll="{ x: 860 }"
      :bordered="false"
      class="table-tight"
    >
      <template #columns>
        <a-table-column :title="t('时间', 'Time')" :width="170"
          ><template #cell="{ record }"
            ><span class="history-time">{{ record.at ? fmtDateTime(record.at) : '—' }}</span></template
          ></a-table-column
        >
        <a-table-column v-if="view === 'all'" :title="t('平台', 'Platform')" :width="130"
          ><template #cell="{ record }"
            ><span class="history-platform"><ProviderMark :provider="record.provider" />{{ providerLabel(record.provider) }}</span></template
          ></a-table-column
        >
        <a-table-column :title="t('账号', 'Account')" data-index="account" :ellipsis="true" :tooltip="true" />
        <a-table-column :title="t('触发方式', 'Trigger')" :width="120"
          ><template #cell="{ record }">{{ triggerName(record.trigger) }}</template></a-table-column
        >
        <a-table-column :title="t('操作', 'Action')" data-index="action" :width="100" />
        <a-table-column :title="t('结果', 'Result')"
          ><template #cell="{ record }"
            ><div class="history-result">
              <span class="state-pill" :class="{ warn: record.tone === 'warn', bad: record.tone === 'bad', off: record.tone === 'off' }">{{
                record.result
              }}</span>
              <b v-if="record.credits">+{{ fmtNumber(record.credits) }}</b>
              <small v-if="record.detail" class="muted" :title="record.detail">{{ record.detail }}</small>
            </div></template
          ></a-table-column
        >
      </template>
    </a-table>
  </section>
</template>
<style scoped>
.history-views {
  display: inline-flex;
  flex-wrap: wrap;
  align-items: center;
  max-width: 100%;
  margin-bottom: 14px;
}
.history-views :deep(.arco-radio-button) {
  display: inline-flex;
  align-items: center;
  height: 30px;
}
.history-views :deep(.arco-radio-button-content) {
  display: inline-flex;
  align-items: center;
  gap: 6px;
  height: 30px;
  line-height: 30px;
  white-space: nowrap;
}
.history-views :deep(.pmark) {
  width: 16px;
  height: 16px;
  border-radius: 5px;
}
.history-time {
  white-space: nowrap;
}
.history-count {
  font-size: 11px;
  color: var(--muted);
  font-variant-numeric: tabular-nums;
}
.history-platform {
  display: inline-flex;
  align-items: center;
  gap: 6px;
  white-space: nowrap;
}
.history-result {
  display: flex;
  align-items: center;
  gap: 8px;
  min-width: 0;
}
.history-result b {
  font-size: 12px;
  color: var(--accent);
  white-space: nowrap;
}
.history-result small {
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
</style>
