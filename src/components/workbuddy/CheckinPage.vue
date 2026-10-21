<script setup lang="ts">
import { t, fmtShort } from '../../i18n'
import { computed, ref, watch } from 'vue'
import {
  IconCalendar,
  IconCheck,
  IconRefresh,
  IconSearch,
  IconSettings,
} from '@arco-design/web-vue/es/icon'
import type { Account, Snapshot, CheckinConfig } from '../../wb-api'
import { accountName, credits } from '../../lib/account-display'
import { dateKey } from '../../lib/statistics'
import { checkinStatus, freshCheckin, windowIssue } from '../../lib/checkin'
const props = defineProps<{
  snapshot: Snapshot
  pendingIds: string[]
  now: number
  progress: { completed: number; total: number; failed: number }
}>()
const emit = defineEmits<{
  toggleAuto: [enabled: boolean]
  toggleAccount: [account: Account, enabled: boolean]
  checkin: [account: Account, submit: boolean]
  batch: [submit: boolean]
  saveConfig: [config: CheckinConfig]
}>()
const search = ref(''),
  filter = ref('all'),
  settingsOpen = ref(false),
  historyOpen = ref(false)
const start = ref('09:00'),
  end = ref('24:00'),
  dirty = ref(false)
watch(
  () => props.snapshot.checkinConfig,
  (config) => {
    if (config && !dirty.value) {
      start.value = config.start
      end.value = config.end
    }
  },
  { immediate: true }
)
const issue = computed(() => windowIssue(start.value, end.value))
const busy = (id: string) =>
  props.pendingIds.includes(id) ||
  props.snapshot.accounts.some((a) => a.id === id && a.checkinRunning)
const batchBusy = computed(() => busy('checkin-all'))
const accounts = computed(() => props.snapshot.accounts.filter((a) => a.variant === 'cn'))
const status = (a: Account) => checkinStatus(a, props.now)
const labels = computed(() => ({
  done: t('今日已签到', 'Checked in'),
  pending: t('今日待签到', 'Pending'),
  inactive: t('活动未开放', 'Not open'),
  error: t('操作失败', 'Failed'),
  unknown: t('今日未查询', 'Not queried'),
}))
const done = computed(() => accounts.value.filter((a) => status(a) === 'done').length)
const enabled = computed(() => accounts.value.filter((a) => a.checkinEnabled).length)
const failed = computed(() => accounts.value.filter((a) => status(a) === 'error').length)
const reward = computed(() =>
  accounts.value
    .filter((a) => freshCheckin(a, props.now))
    .reduce((n, a) => n + (a.checkin?.today_credit || 0), 0)
)
const tabs = computed(() => [
  { id: 'all', label: t('全部账号', 'All'), count: accounts.value.length },
  ...(['done', 'pending', 'error', 'unknown', 'inactive'] as const).map((id) => ({
    id,
    label: {
      done: t('已签到', 'Done'),
      pending: t('待签到', 'Pending'),
      error: t('失败', 'Failed'),
      unknown: t('未查询', 'Not queried'),
      inactive: t('未开放', 'Not open'),
    }[id],
    count: accounts.value.filter((a) => status(a) === id).length,
  })),
  { id: 'enabled', label: t('已开启', 'Enabled'), count: enabled.value },
])
const shown = computed(() =>
  accounts.value.filter(
    (a) =>
      (!search.value.trim() ||
        [accountName(a), a.presentation?.uid, a.presentation?.phoneNumber, a.note].some((v) =>
          v?.toLowerCase().includes(search.value.trim().toLowerCase())
        )) &&
      (filter.value === 'all' ||
        status(a) === filter.value ||
        (filter.value === 'enabled' && a.checkinEnabled))
  )
)
const records = computed(() =>
  [...(props.snapshot.checkinRecords || [])]
    .filter((r) => r.at >= props.now - 30 * 86400000)
    .reverse()
)
const name = (id: string) => {
  const a = props.snapshot.accounts.find((a) => a.id === id)
  return a ? accountName(a) : t('已移除账号', 'Removed account')
}
const time = (v?: number) => (v ? fmtShort(v) : '—')
function clearWindow() {
  start.value = ''
  end.value = ''
  dirty.value = true
}
function save() {
  if (issue.value) return
  emit('saveConfig', { start: start.value, end: end.value })
  // Keep an unsaved draft across snapshot polling; clear only after persisted values match.
}
watch(
  () => props.snapshot.checkinConfig,
  (config) => {
    if (config?.start === start.value && config.end === end.value) dirty.value = false
  }
)
const configuredWindow = computed(() => {
  const c = props.snapshot.checkinConfig
  return c?.start && c.end ? `${c.start}–${c.end}` : t('全天', 'all day')
})
</script>
<template>
  <section class="checkin-summary">
    <div class="summary-heading">
      <div>
        <span class="eyebrow"><IconCalendar /> DAILY REWARDS · {{ dateKey(now) }}</span>
        <h2>{{ t('每日签到', 'Daily check-in') }}</h2>
        <p>{{ t('官方状态核验 · 账号独立控制 · 上海时间', 'Official status checks · per-account control · Shanghai time') }}</p>
      </div>
      <div class="auto-setting">
        <div>
          <strong>{{ t('自动签到', 'Auto check-in') }}</strong
          ><small>{{
            snapshot.autoCheckin ? t(configuredWindow + ' 内分散执行', 'Spread within ' + configuredWindow) : t('已关闭，默认不自动提交', 'Off; nothing is submitted automatically')
          }}</small>
        </div>
        <a-switch
          :model-value="snapshot.autoCheckin ?? false"
          :loading="busy('auto-checkin')"
          :aria-label="t('启用自动签到', 'Enable auto check-in')"
          @change="(v) => emit('toggleAuto', !!v)"
        />
      </div>
    </div>
    <div class="summary-metrics">
      <div>
        <span>{{ t('今日已签到', 'Checked in today') }}</span
        ><strong
          >{{ done }}<small> / {{ accounts.length }}</small></strong
        >
      </div>
      <div>
        <span>{{ t('今日奖励（已查询）', 'Rewards today (queried)') }}</span><strong>{{ credits(reward) }}<small> {{ t('积分', 'credits') }}</small></strong>
      </div>
      <div>
        <span>{{ t('参与批量 / 自动签到', 'In batch / auto') }}</span><strong>{{ enabled }}<small> {{ t('个账号', 'accounts') }}</small></strong>
      </div>
      <div>
        <span>{{ t('今日操作失败', 'Failed today') }}</span
        ><strong :class="{ 'error-text': failed }">{{ failed }}<small> {{ t('个账号', 'accounts') }}</small></strong>
      </div>
    </div>
    <div class="summary-bottom">
      <span><IconCheck /> {{ t('先查询再签到；签到与反代开关独立，国际版不参与。', 'Status is queried before checking in; independent of the proxy switch; international accounts are excluded.') }}</span>
      <div>
        <a-button size="small" @click="settingsOpen = !settingsOpen"
          ><template #icon><IconSettings /></template>{{ t('签到设置', 'Settings') }}</a-button
        >
        <a-button size="small" @click="historyOpen = !historyOpen">{{ t('操作记录', 'History') }}</a-button>
      </div>
    </div>
  </section>
  <section v-if="settingsOpen" class="checkin-settings">
    <div>
      <h3>{{ t('自动签到时间段', 'Auto check-in window') }}</h3>
      <p>{{ t('每天按账号分散执行；同时留空为全天，不支持跨午夜。应用必须运行。', 'Spread across accounts each day; leave both empty for all day; no midnight crossing. The app must be running.') }}</p>
    </div>
    <div class="window-fields">
      <a-input
        v-model="start"
        placeholder="09:00"
        :aria-label="t('签到开始时间', 'Window start')"
        :max-length="5"
        @input="dirty = true"
      />
      <span>{{ t('至', 'to') }}</span
      ><a-input
        v-model="end"
        placeholder="24:00"
        :aria-label="t('签到结束时间', 'Window end')"
        :max-length="5"
        @input="dirty = true"
      />
      <a-button
        :disabled="!dirty || !!issue"
        :loading="busy('checkin-settings')"
        type="primary"
        @click="save"
        >{{ t('保存时间段', 'Save window') }}</a-button
      >
      <a-button @click="clearWindow">{{ t('不限时间', 'All day') }}</a-button>
    </div>
    <p v-if="issue" class="error-text">{{ issue }}</p>
    <p>
      {{ t('失败后每 5 分钟重试，最多连续 12 次后降为每 30 分钟核验；成功后每 30 分钟补偿检查，已签到不会再次提交。', 'On failure it retries every 5 minutes, then every 30 minutes after 12 attempts; after success it re-checks every 30 minutes and never submits twice.') }}
    </p>
  </section>
  <div class="checkin-toolbar">
    <div class="checkin-tabs">
      <button
        v-for="tab in tabs"
        :key="tab.id"
        :class="{ active: filter === tab.id }"
        @click="filter = tab.id"
      >
        {{ tab.label }}<span>{{ tab.count }}</span>
      </button>
    </div>
    <div class="checkin-actions">
      <a-input-search
        v-model="search"
        :placeholder="t('搜索账号 / 备注', 'Search account / note')"
        allow-clear
        :aria-label="t('搜索签到账号', 'Search accounts')"
      />
      <a-button
        :disabled="batchBusy || !accounts.length"
        :loading="batchBusy"
        @click="emit('batch', false)"
      >
        <template #icon><IconRefresh /></template>{{ t('查询全部状态', 'Query all') }}</a-button
      >
      <a-button
        type="primary"
        :disabled="batchBusy || !enabled"
        :loading="batchBusy"
        @click="emit('batch', true)"
        >{{ t('已开启账号签到', 'Check in enabled accounts') }}</a-button
      >
    </div>
  </div>
  <div v-if="batchBusy" class="batch-progress" role="status" aria-live="polite">
    <span
      >{{ t('正在处理', 'Processing') }} {{ progress.completed }} / {{ progress.total }} · {{ t('失败', 'failed') }} {{ progress.failed }}</span
    >
    <a-progress
      :percent="progress.total ? progress.completed / progress.total : 0"
      :show-text="false"
    />
  </div>
  <div class="checkin-grid">
    <article
      v-for="a in shown"
      :key="a.id"
      class="checkin-account"
      :class="{ 'card-done': status(a) === 'done' }"
    >
      <header class="ca-heading">
        <div class="ca-avatar">{{ accountName(a).slice(0, 1) }}</div>
        <div class="ca-identity">
          <h3 :title="accountName(a)">{{ accountName(a) }}</h3>
          <span
            >{{ a.checkinEnabled ? t('参与签到', 'Check-in on') : t('自动 / 批量签到已关闭', 'Auto / batch off') }} ·
            {{ a.enabled ? t('反代开启', 'Proxy on') : t('反代关闭', 'Proxy off') }}</span
          >
        </div>
        <a-tag
          :color="
            status(a) === 'done'
              ? 'green'
              : status(a) === 'error'
                ? 'red'
                : status(a) === 'pending'
                  ? 'orange'
                  : 'gray'
          "
          >{{ labels[status(a)] }}</a-tag
        >
      </header>
      <div class="ca-metrics">
        <div>
          <span>{{ t('连续签到', 'Streak') }}</span
          ><strong
            >{{ freshCheckin(a, now) ? (a.checkin?.streak_days ?? '—') : '—'
            }}<small>{{ t('天', 'd') }}</small></strong
          >
        </div>
        <div>
          <span>{{ t('今日奖励', 'Today') }}</span
          ><strong
            >{{ freshCheckin(a, now) ? credits(a.checkin?.today_credit) : '—'
            }}<small>{{ t('积分', 'credits') }}</small></strong
          >
        </div>
      </div>
      <p class="ca-hint" :class="{ 'error-text': status(a) === 'error' }">
        {{
          status(a) === 'error'
            ? a.checkinError
            : status(a) === 'unknown'
              ? t('今日尚未查询，历史状态不计入今日汇总。', 'Not queried today; past status is not counted.')
              : status(a) === 'inactive'
                ? t('官方活动尚未开放，不会提交签到。', 'The official campaign is not open; nothing is submitted.')
                : status(a) === 'done'
                  ? t('今日奖励已领取，明天继续。', 'Claimed today; see you tomorrow.')
                  : t('可领取今日奖励，提交前会再次核验官方状态。', 'Reward available; the official status is re-checked before submitting.')
        }}
      </p>
      <footer class="ca-footer">
        <label
          >{{ t('参与自动 / 批量签到', 'Auto / batch') }}
          <a-switch
            size="small"
            :model-value="a.checkinEnabled"
            :loading="busy(a.id)"
            :aria-label="accountName(a) + t('参与签到', ' check-in')"
            @change="(v) => emit('toggleAccount', a, !!v)"
        /></label>
        <span :title="a.checkinUpdatedAt ? new Date(a.checkinUpdatedAt).toISOString() : ''"
          >{{ t('更新', 'Updated') }} {{ time(a.checkinUpdatedAt) }}</span
        >
      </footer>
      <div class="ca-buttons">
        <a-button :disabled="busy(a.id)" :loading="busy(a.id)" @click="emit('checkin', a, false)"
          ><template #icon><IconSearch /></template>{{ t('查询状态', 'Query') }}</a-button
        >
        <a-button
          type="primary"
          :disabled="
            busy(a.id) || !a.checkinEnabled || status(a) === 'done' || status(a) === 'inactive'
          "
          @click="emit('checkin', a, true)"
          >{{
            status(a) === 'done'
              ? t('今日已领取', 'Claimed today')
              : !a.checkinEnabled
                ? t('请先开启签到', 'Enable check-in first')
                : status(a) === 'inactive'
                  ? t('活动未开放', 'Not open')
                  : t('查询并签到', 'Check in')
          }}</a-button
        >
      </div>
    </article>
  </div>
  <a-empty v-if="!shown.length" class="empty-panel" :description="t('没有符合条件的国内版账号', 'No matching China accounts')" />
  <section v-if="historyOpen" class="checkin-history">
    <h3>{{ t('签到操作记录', 'Check-in history') }} <small>{{ t('最近 30 天，最多 1,000 条 · 仅本机保存', 'Last 30 days, up to 1,000 · stored locally') }}</small></h3>
    <a-empty v-if="!records.length" :description="t('暂无操作记录', 'No records')" />
    <div v-else class="history-scroll">
      <table>
        <thead>
          <tr>
            <th>{{ t('时间', 'Time') }}</th>
            <th>{{ t('账号', 'Account') }}</th>
            <th>{{ t('操作', 'Action') }}</th>
            <th>{{ t('结果', 'Result') }}</th>
          </tr>
        </thead>
        <tbody>
          <tr v-for="(r, i) in records" :key="r.at + ':' + i">
            <td>{{ time(r.at) }}</td>
            <td>{{ name(r.accountId) }}</td>
            <td>{{ r.submit ? t('核验并签到', 'Check in') : t('查询状态', 'Query') }}</td>
            <td :class="{ 'error-text': r.error }">
              {{
                r.error ||
                (r.status?.today_checked_in
                  ? t('今日已签到', 'Checked in')
                  : r.status?.active === false
                    ? t('活动未开放', 'Not open')
                    : t('待签到', 'Pending'))
              }}
            </td>
          </tr>
        </tbody>
      </table>
    </div>
  </section>
  <p class="checkin-footnote">
    {{ t('关闭页面不停止自动任务，退出应用后停止。手动批量立即执行，不受自动签到时间段限制。', 'Leaving the page keeps auto tasks running until the app quits. Manual batches run immediately, outside the window.') }}
    {{ t(`国际版 ${snapshot.accounts.filter((a) => a.variant === 'ai').length} 个账号不参与此接口。`, `${snapshot.accounts.filter((a) => a.variant === 'ai').length} international accounts are excluded.`) }}
  </p>
</template>
<style scoped>
.checkin-summary {
  border: 1px solid var(--c-dcebe2);
  border-radius: 18px;
  padding: 24px;
  background: linear-gradient(110deg, var(--c-eef9f3), var(--c-fbfdfb));
}
.summary-heading,
.summary-bottom,
.auto-setting,
.window-fields,
.checkin-toolbar,
.checkin-actions {
  display: flex;
  align-items: center;
  gap: 12px;
  justify-content: space-between;
}
.eyebrow {
  display: flex;
  gap: 7px;
  align-items: center;
  font-size: 11px;
  letter-spacing: 1px;
  color: var(--c-458c6b);
}
h2 {
  margin: 10px 0 7px;
  font-size: 25px;
  color: var(--c-263e30);
}
h3 {
  margin: 0;
  font-size: 14px;
}
.summary-heading p,
.checkin-settings p {
  margin: 7px 0;
  color: var(--c-788e80);
  font-size: 12px;
  line-height: 1.7;
}
.auto-setting {
  background: color-mix(in srgb, var(--surface) 72%, transparent);
  padding: 13px 18px;
  border: 1px solid var(--c-dfebe3);
  border-radius: 12px;
  gap: 24px;
}
.auto-setting strong {
  font-size: 13px;
}
.auto-setting small {
  display: block;
  color: var(--c-789183);
  font-size: 11px;
  margin-top: 5px;
}
.summary-metrics {
  display: grid;
  grid-template-columns: repeat(4, minmax(0, 1fr));
  gap: 20px;
  padding: 22px 0;
  margin-top: 18px;
  border-top: 1px solid var(--c-deebe2);
}
.summary-metrics span {
  font-size: 12px;
  color: var(--c-718b79);
}
.summary-metrics strong {
  display: block;
  font-size: 28px;
  margin-top: 8px;
  color: var(--c-2e4b39);
  font-variant-numeric: tabular-nums;
}
.summary-metrics small {
  font-size: 12px;
  font-weight: 400;
  color: var(--c-799083);
}
.summary-bottom {
  border-top: 1px solid var(--c-deebe2);
  padding-top: 15px;
  flex-wrap: wrap;
  font-size: 12px;
  color: var(--c-658770);
}
.summary-bottom > div {
  display: flex;
  gap: 8px;
}
.summary-bottom > span {
  display: flex;
  align-items: center;
  gap: 7px;
}
.checkin-settings,
.checkin-history {
  background: var(--surface);
  border: 1px solid var(--c-dfe8e2);
  border-radius: 14px;
  padding: 20px;
  margin: 16px 0;
}
.window-fields {
  justify-content: flex-start;
  flex-wrap: wrap;
  margin: 14px 0;
}
.window-fields .arco-input-wrapper {
  width: 105px;
}
.checkin-toolbar {
  flex-wrap: wrap;
  margin: 22px 0 16px;
}
.checkin-tabs {
  display: flex;
  gap: 3px;
  flex-wrap: wrap;
}
.checkin-tabs button {
  border: 0;
  background: transparent;
  color: var(--c-718479);
  padding: 8px 10px;
  border-radius: 8px;
  cursor: pointer;
  font-size: 12px;
}
.checkin-tabs button span {
  font-size: 10px;
  margin-left: 5px;
  opacity: 0.75;
}
.checkin-tabs .active {
  background: var(--c-e4f3eb);
  color: var(--c-217d58);
}
.checkin-actions {
  flex-wrap: wrap;
}
.checkin-actions .arco-input-search {
  width: 185px;
}
.batch-progress {
  padding: 12px 16px;
  border-radius: 10px;
  background: var(--c-e9f5ee);
  color: var(--c-377254);
  font-size: 12px;
  margin-bottom: 16px;
}
.batch-progress .arco-progress {
  margin-top: 7px;
}
.checkin-grid {
  display: grid;
  grid-template-columns: repeat(3, minmax(0, 1fr));
  gap: 16px;
}
.checkin-account {
  background: var(--surface);
  border: 1px solid var(--c-e1e7e3);
  border-radius: 15px;
  padding: 20px;
  box-shadow: 0 2px 5px #24422f04;
  min-width: 0;
}
.card-done {
  border-color: var(--c-c9e7d8);
}
.ca-heading {
  display: flex;
  align-items: center;
  gap: 10px;
}
.ca-avatar {
  width: 40px;
  height: 40px;
  display: grid;
  place-items: center;
  flex-shrink: 0;
  border-radius: 12px;
  background: var(--c-e5f5ed);
  color: var(--c-279a70);
  font-size: 19px;
}
.ca-identity {
  min-width: 0;
  flex: 1;
}
.ca-identity h3 {
  overflow: hidden;
  white-space: nowrap;
  text-overflow: ellipsis;
  color: var(--c-34443b);
  margin-bottom: 5px;
}
.ca-identity span {
  color: var(--c-829389);
  font-size: 10px;
}
.ca-heading .arco-tag {
  font-size: 10px;
  flex-shrink: 0;
  padding: 0 6px;
}
.ca-metrics {
  display: grid;
  grid-template-columns: 1fr 1fr;
  gap: 16px;
  margin: 24px 0 14px;
}
.ca-metrics span {
  font-size: 11px;
  color: var(--c-81958a);
}
.ca-metrics strong {
  display: block;
  color: var(--c-3b5143);
  font-size: 25px;
  margin-top: 7px;
  font-weight: 600;
}
.ca-metrics small {
  color: var(--c-859b8d);
  font-size: 11px;
  font-weight: 400;
  margin-left: 6px;
}
.ca-hint {
  color: var(--c-82958a);
  font-size: 11px;
  line-height: 1.7;
  min-height: 38px;
  margin: 0 0 12px;
  overflow-wrap: anywhere;
}
.ca-footer {
  border-top: 1px solid var(--c-edf1ee);
  padding-top: 14px;
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  justify-content: space-between;
  gap: 10px;
  color: var(--c-82958a);
  font-size: 10px;
}
.ca-footer label {
  display: flex;
  align-items: center;
  gap: 9px;
}
.ca-buttons {
  display: flex;
  gap: 9px;
  margin-top: 15px;
}
.ca-buttons > * {
  flex: 1;
  min-width: 0;
}
.error-text,
.summary-metrics .error-text {
  color: var(--c-b24e42);
}
.checkin-history h3 small {
  font-size: 11px;
  font-weight: 400;
  color: var(--c-82958a);
  margin-left: 10px;
}
.history-scroll {
  max-height: 340px;
  overflow: auto;
  margin-top: 15px;
}
table {
  width: 100%;
  border-collapse: collapse;
  text-align: left;
  font-size: 12px;
}
th,
td {
  padding: 10px;
  border-bottom: 1px solid var(--c-edf1ee);
}
th {
  color: var(--c-82958a);
  position: sticky;
  top: 0;
  background: var(--surface);
}
td:last-child {
  overflow-wrap: anywhere;
}
.checkin-footnote {
  font-size: 11px;
  color: var(--c-82958a);
  line-height: 1.8;
  margin: 20px 0;
}
@media (max-width: 1350px) {
  .checkin-grid {
    grid-template-columns: repeat(2, minmax(0, 1fr));
  }
}
@media (max-width: 850px) {
  .checkin-grid {
    grid-template-columns: 1fr;
  }
  .summary-heading {
    align-items: flex-start;
    flex-direction: column;
  }
  .summary-metrics {
    grid-template-columns: repeat(2, minmax(0, 1fr));
  }
  .checkin-summary {
    padding: 18px;
  }
  .checkin-actions {
    width: 100%;
    justify-content: flex-start;
  }
  .ca-heading .arco-tag {
    white-space: normal;
  }
}
</style>
