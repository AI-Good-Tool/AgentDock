<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref, watch } from 'vue'
import { Modal } from '@arco-design/web-vue'
import { IconDelete, IconRefresh, IconFolder } from '@arco-design/web-vue/es/icon'
import PageHead from '../components/PageHead.vue'
import { call, type AppLogEntry } from '../api'
import { t, fmtDateTime } from '../i18n'
import { state, act, busy, saveSettings } from '../store'

const level = ref('debug')
const source = ref('')
const query = ref('')
const live = ref(true)
const data = ref<{ entries: AppLogEntry[]; total: number; sources: string[]; file: string; enabled: boolean }>({
  entries: [],
  total: 0,
  sources: [],
  file: '',
  enabled: true,
})
const open = ref(new Set<string>())
const enabled = computed(() => state.app?.settings.logging ?? true)

const sourceNames: Record<string, [string, string]> = {
  app: ['应用', 'App'],
  gateway: ['网关请求', 'Gateway'],
  internal: ['内部操作', 'Internal'],
  workbuddy: ['WorkBuddy', 'WorkBuddy'],
  'workbuddy.sdk': ['WorkBuddy 设备 SDK', 'WorkBuddy SDK'],
  'workbuddy.agent': ['WorkBuddy Agent', 'WorkBuddy Agent'],
  checkin: ['签到', 'Check-in'],
  'checkin.workbuddy': ['签到 · WorkBuddy', 'Check-in · WorkBuddy'],
  'checkin.qoder': ['领取 · Qoder', 'Claim · Qoder'],
  'checkin.trae': ['签到 · Trae', 'Check-in · Trae'],
  trae: ['Trae 运行事件', 'Trae activity'],
  backup: ['备份', 'Backup'],
}
const sourceLabel = (s: string) => (sourceNames[s] ? t(...sourceNames[s]) : s)
const sourceOptions = computed(() => {
  // Offer each group prefix ("workbuddy", "checkin") as well as the exact sources seen.
  const set = new Set<string>()
  for (const s of data.value.sources) {
    set.add(s)
    if (s.includes('.')) set.add(s.split('.')[0])
  }
  return [...set].sort()
})
const levelName: Record<string, [string, string]> = {
  debug: ['调试', 'Debug'],
  info: ['信息', 'Info'],
  warn: ['警告', 'Warn'],
  error: ['错误', 'Error'],
}
const key = (e: AppLogEntry) => String(e.seq)
const hasFields = (e: AppLogEntry) => e.fields && typeof e.fields === 'object' && Object.keys(e.fields).length > 0

async function load() {
  try {
    data.value = await call('app_logs', { filter: { level: level.value, source: source.value, query: query.value, limit: 500 } })
  } catch {
    /* desktop-only */
  }
}
watch([level, source], load)
let searchTimer: number | undefined
watch(query, () => {
  clearTimeout(searchTimer)
  searchTimer = window.setTimeout(load, 250)
})
let timer: number | undefined
onMounted(() => {
  load()
  timer = window.setInterval(() => {
    if (live.value && enabled.value && !document.hidden) load()
  }, 3000)
})
onBeforeUnmount(() => {
  clearInterval(timer)
  clearTimeout(searchTimer)
})
function toggle(k: string) {
  const next = new Set(open.value)
  next.has(k) ? next.delete(k) : next.add(k)
  open.value = next
}
function setEnabled(on: boolean) {
  saveSettings({ logging: on }, on ? t('已开启运行日志', 'Runtime log on') : t('已关闭运行日志', 'Runtime log off')).then(load)
}
function clear() {
  Modal.confirm({
    title: t('清空运行日志', 'Clear runtime log'),
    content: t('内存中的日志与日志文件都会被清空，不可恢复。', 'Clears the in-memory log and the log file. This cannot be undone.'),
    onOk: () => act('logs-clear', () => call('clear_app_logs'), t('已清空', 'Cleared'), [load]),
  })
}
const time = (ms: number) => fmtDateTime(ms)
</script>

<template>
  <PageHead
    eyebrow="RUNTIME LOG"
    :title="t('运行日志', 'Runtime log')"
    :desc="t('网关请求、各平台调用、WorkBuddy 设备 SDK 与 Agent 调用、Trae 刷新与签到等运行事件、备份。可按来源筛选；关闭后不记录。', 'Gateway requests, provider calls, WorkBuddy device SDK and Agent calls, Trae refresh and check-in activity, and backups. Filter by source; nothing is recorded while off.')"
  >
    <div class="log-switch">
      <span>{{ t('记录日志', 'Record') }}</span>
      <a-switch :model-value="enabled" :loading="busy('settings')" @change="(v) => setEnabled(!!v)" />
    </div>
  </PageHead>

  <section class="box log-box">
    <div class="log-toolbar">
      <a-radio-group v-model="level" type="button" size="small">
        <a-radio value="debug">{{ t('全部', 'All') }}</a-radio>
        <a-radio value="info">{{ t('信息+', 'Info+') }}</a-radio>
        <a-radio value="warn">{{ t('警告+', 'Warn+') }}</a-radio>
        <a-radio value="error">{{ t('错误', 'Error') }}</a-radio>
      </a-radio-group>
      <a-select v-model="source" size="small" style="width: 200px" :placeholder="t('全部来源', 'All sources')" allow-clear>
        <a-option value="">{{ t('全部来源', 'All sources') }}</a-option>
        <a-option v-for="s in sourceOptions" :key="s" :value="s">{{ sourceLabel(s) }}</a-option>
      </a-select>
      <a-input-search v-model="query" size="small" style="width: 220px" :placeholder="t('搜索消息 / 账号 / 模型', 'Search message / account / model')" allow-clear />
      <span class="spacer"></span>
      <label class="live"><a-switch v-model="live" size="small" />{{ t('实时刷新', 'Live') }}</label>
      <a-button size="small" @click="load"><template #icon><IconRefresh /></template>{{ t('刷新', 'Refresh') }}</a-button>
      <a-button size="small" status="danger" :loading="busy('logs-clear')" @click="clear"><template #icon><IconDelete /></template>{{ t('清空', 'Clear') }}</a-button>
    </div>

    <div v-if="!enabled" class="log-off">
      <b>{{ t('运行日志已关闭', 'Runtime log is off') }}</b>
      <span>{{ t('打开右上角「记录日志」后开始记录；关闭期间不产生任何开销。', 'Turn on “Record” to start; while off nothing is recorded and there is no overhead.') }}</span>
    </div>
    <div v-else class="log-list">
      <div v-for="e in data.entries" :key="key(e)" class="log-row" :class="[e.level, { open: open.has(key(e)), expandable: hasFields(e) }]" @click="hasFields(e) && toggle(key(e))">
        <span class="log-time mono">{{ time(e.time) }}</span>
        <span class="log-level">{{ t(...levelName[e.level]) }}</span>
        <span class="log-source">{{ sourceLabel(e.source) }}</span>
        <span class="log-message">{{ e.message }}</span>
        <pre v-if="open.has(key(e))" class="log-fields mono" @click.stop>{{ JSON.stringify(e.fields, null, 2) }}</pre>
      </div>
      <a-empty v-if="!data.entries.length" :description="t('暂无日志', 'No log entries')" />
    </div>
    <div class="log-foot">
      <span>{{ t(`显示 ${data.entries.length} 条 · 内存保留最近 ${data.total} 条（上限 3000）`, `Showing ${data.entries.length} · ${data.total} kept in memory (max 3000)`) }}</span>
      <span v-if="data.file" class="mono"><IconFolder /> {{ data.file }}</span>
    </div>
    <p class="hint">{{ t('只记录摘要（模型、账号名、状态、耗时、Agent 用途），不保存提示词、生成内容或令牌。日志文件超过 5 MB 自动轮转。', 'Only summaries are kept (model, account name, status, timing, Agent purpose) — never prompts, outputs or tokens. The file rotates at 5 MB.') }}</p>
  </section>
</template>

<style>
.log-switch {
  white-space: nowrap;
  display: flex;
  align-items: center;
  gap: 10px;
  font-size: 13px;
  color: var(--text-2);
}
.log-toolbar {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: 10px;
  padding-bottom: 14px;
  border-bottom: 1px solid var(--line);
}
.log-toolbar .spacer {
  flex: 1;
}
.log-toolbar .live {
  display: inline-flex;
  align-items: center;
  gap: 6px;
  font-size: 12px;
  color: var(--muted);
}
.log-off {
  display: flex;
  flex-direction: column;
  gap: 6px;
  padding: 36px 0;
  text-align: center;
  color: var(--muted);
  font-size: 13px;
}
.log-off b {
  color: var(--text);
  font-size: 14px;
}
.log-list {
  max-height: calc(100vh - 360px);
  min-height: 240px;
  overflow: auto;
}
.log-row {
  display: grid;
  grid-template-columns: 150px 52px 168px 1fr;
  gap: 12px;
  align-items: baseline;
  padding: 8px 6px;
  border-bottom: 1px solid var(--line);
  font-size: 12.5px;
}
.log-row.expandable {
  cursor: pointer;
}
.log-row.expandable:hover {
  background: var(--surface-2);
}
.log-time {
  color: var(--muted);
  font-size: 11.5px;
}
.log-level {
  font-size: 11px;
  font-weight: 600;
  text-align: center;
  border-radius: 4px;
  padding: 1px 0;
  background: var(--surface-2);
  color: var(--muted);
}
.log-row.info .log-level {
  background: var(--accent-soft);
  color: var(--accent);
}
.log-row.warn .log-level {
  background: var(--warn-soft);
  color: var(--warn);
}
.log-row.error .log-level {
  background: var(--danger-soft);
  color: var(--danger);
}
.log-source {
  color: var(--text-2);
  font-weight: 500;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.log-message {
  color: var(--text);
  word-break: break-word;
}
.log-row.error .log-message {
  color: var(--danger);
}
.log-fields {
  grid-column: 2 / -1;
  margin: 4px 0 2px;
  padding: 10px 12px;
  background: var(--surface-2);
  border: 1px solid var(--line);
  border-radius: 8px;
  font-size: 11.5px;
  line-height: 1.6;
  white-space: pre-wrap;
  overflow-wrap: anywhere;
  cursor: text;
}
.log-foot {
  display: flex;
  justify-content: space-between;
  gap: 12px;
  flex-wrap: wrap;
  padding-top: 12px;
  font-size: 12px;
  color: var(--muted);
}
.log-foot .mono {
  display: inline-flex;
  align-items: center;
  gap: 6px;
}
</style>
