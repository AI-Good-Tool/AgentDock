<script setup lang="ts">
import { computed, ref, onMounted } from 'vue'
import { Message } from '@arco-design/web-vue'
import PageHead from '../components/PageHead.vue'
import { call } from '../api'
import { t, lang, systemLang, systemZone, zoneLabel, zoneOptions, defaultZone, zone, followSystemZone, fmtDateTime, langSetting, tzSetting } from '../i18n'
import { state, act, busy, saveSettings, refreshPool, themeSetting } from '../store'

const launchd = ref<{ installed: boolean; path?: string } | null>(null)
onMounted(async () => {
  launchd.value = await call('launchd_status').catch(() => null)
  refreshPool()
})
const zones = computed(() => zoneOptions())

function setLang(v: string) {
  langSetting.value = v as any
  saveSettings({ lang: v })
}
function setFollow(on: boolean) {
  const next = on ? 'system' : defaultZone(lang.value)
  tzSetting.value = next
  saveSettings({ timezone: next })
}
function setZone(v: string) {
  tzSetting.value = v
  saveSettings({ timezone: v })
}
function setTheme(v: string) {
  themeSetting.value = v as any
  saveSettings({ theme: v })
}
function setTokenFormat(v: string) {
  saveSettings({ tokenFormat: v })
}
// Qoder data
const qoderAuto = computed(() => state.pool?.qoder?.settings?.autoClaim !== false)
function exportQoder() {
  return act('qexport', () => call('qoder_export_accounts'))
}
const importPath = ref<string | null>(null)
async function importQoder() {
  const path = (importPath.value || '').trim()
  if (!path) return Message.error(t('请输入备份文件路径', 'Enter the backup file path'))
  const r = await act('qimport', () => call('qoder_import_accounts', { path }))
  if (r) importPath.value = null
}
function toggleLaunchd() {
  const install = !launchd.value?.installed
  return act(
    'launchd',
    async () => {
      const r = await call('launchd_install', { install })
      launchd.value = await call('launchd_status')
      return r
    },
    install ? t('已安装每日自动领取：每天 10:05 和 22:05 运行（无需打开应用）', 'Installed: runs daily at 10:05 and 22:05 without opening the app') : t('已移除系统定时任务', 'Scheduled task removed'),
    []
  )
}
const migration = computed(() => state.app?.migration)
</script>

<template>
  <PageHead eyebrow="PREFERENCES" :title="t('偏好设置', 'Preferences')" :desc="t('外观、语言、时区与数据管理。', 'Appearance, language, time zone and data.')" />
  <section class="box">
    <div class="box-head"><h2>{{ t('外观', 'Appearance') }}</h2></div>
    <div class="field-row">
      <div><b>{{ t('主题', 'Theme') }}</b><small>{{ t('立即生效并自动记住', 'Applies instantly and is remembered') }}</small></div>
      <div class="control">
        <a-radio-group :model-value="themeSetting" type="button" @change="(v) => setTheme(String(v))">
          <a-radio value="light">{{ t('浅色', 'Light') }}</a-radio>
          <a-radio value="dark">{{ t('深色', 'Dark') }}</a-radio>
          <a-radio value="system">{{ t('跟随系统', 'System') }}</a-radio>
        </a-radio-group>
      </div>
    </div>
    <div class="field-row">
      <div><b>{{ t('Token 数字格式', 'Token number format') }}</b></div>
      <div class="control">
        <a-radio-group :model-value="state.app?.settings.tokenFormat ?? 'km'" type="button" @change="(v) => setTokenFormat(String(v))">
          <a-radio value="km">{{ t('K / M 缩写', 'K / M') }}</a-radio>
          <a-radio value="cn">{{ t('万 / 亿', 'Chinese units (万 / 亿)') }}</a-radio>
          <a-radio value="full">{{ t('完整数字', 'Full') }}</a-radio>
        </a-radio-group>
      </div>
    </div>
  </section>

  <section class="box">
    <div class="box-head"><h2>{{ t('语言', 'Language') }}</h2></div>
    <div class="pref-box">
      <div>
        <b>{{ t('跟随系统语言', 'Follow system language') }}</b><small>{{ systemLang() === 'zh' ? '简体中文' : 'English' }}</small>
      </div>
      <a-switch :model-value="langSetting === 'system'" @change="(v) => setLang(v ? 'system' : systemLang())" />
    </div>
    <div class="field-row" style="border-bottom: none">
      <div><b>{{ t('界面语言', 'Display language') }}</b></div>
      <div class="control">
        <a-select :model-value="lang" :disabled="langSetting === 'system'" style="width: 100%" @change="(v) => setLang(String(v))">
          <a-option value="zh">简体中文</a-option>
          <a-option value="en">English</a-option>
        </a-select>
      </div>
    </div>
    <p class="hint">{{ t('开启后跟随 macOS 系统语言：系统为中文时显示中文，其他任何语言一律显示英文；关闭后使用选择的语言。', 'When on, follows the macOS language: Chinese systems show Chinese, every other language shows English. When off, the selected language is used.') }}</p>
  </section>

  <section class="box">
    <div class="box-head"><h2>{{ t('时区', 'Time zone') }}</h2></div>
    <div class="pref-box">
      <div>
        <b>{{ t('跟随系统时区', 'Follow system time zone') }}</b><small>{{ zoneLabel(systemZone()) }}</small>
      </div>
      <a-switch :model-value="followSystemZone" @change="(v) => setFollow(!!v)" />
    </div>
    <div class="field-row" style="border-bottom: none">
      <div><b>{{ t('使用时区', 'Time zone in use') }}</b></div>
      <div class="control">
        <a-select :model-value="zone" :disabled="followSystemZone" allow-search style="width: 100%" @change="(v) => setZone(String(v))">
          <a-option v-for="z in zones" :key="z.value" :value="z.value">{{ z.label }}</a-option>
        </a-select>
      </div>
    </div>
    <p class="hint">
      {{ t('开启后跟随 macOS 系统时区（出差自动切换）；关闭后使用选择的时区，默认中文为北京时间（上海时区）、英文为美国东部（纽约）时区。界面时间、请求记录与 Token 统计的日期分组均按此时区计算（自动处理夏令时）。', 'When on, follows the macOS time zone (switches automatically when you travel). When off, the selected zone is used — Beijing (Shanghai) by default for Chinese, US Eastern (New York) for English. Display times, request logs and token-statistics day boundaries use this zone (DST aware).') }}
      <br />{{ t('当前时间：', 'Now: ') }}{{ fmtDateTime(state.now) }}
    </p>
    <p class="hint">{{ t('官方签到日界线、WorkBuddy 官方积分统计仍按上海时间计算，与官方口径一致。', 'Official check-in days and WorkBuddy official credit statistics still use Shanghai time, matching the official definition.') }}</p>
  </section>

  <section class="box">
    <div class="box-head"><h2>{{ t('每日积分自动领取（Qoder）', 'Daily credit auto-claim (Qoder)') }}</h2></div>
    <div class="field-row">
      <div><b>{{ t('应用内自动领取', 'Auto-claim while running') }}</b><small>{{ t('应用运行时每 10 分钟检查一次可领活动', 'Checks claimable campaigns every 10 minutes') }}</small></div>
      <div class="control"><a-switch :model-value="qoderAuto" :loading="busy('qauto')" @change="(v) => act('qauto', () => call('qoder_set_auto_claim', { enabled: !!v }))" /></div>
    </div>
    <div class="field-row">
      <div><b>{{ t('系统定时任务', 'Scheduled task (launchd)') }}</b><small>{{ t('不开应用也能领：每天 10:05 / 22:05 运行一次领取（Qoder 活动、已开启的 Trae/WorkBuddy 自动签到）', 'Runs daily at 10:05 / 22:05 without the app: Qoder campaigns and opted-in Trae/WorkBuddy check-ins') }}</small></div>
      <div class="control">
        <span class="state-pill" :class="{ off: !launchd?.installed }">{{ launchd?.installed ? t('已安装', 'Installed') : t('未安装', 'Not installed') }}</span>
        <a-button :loading="busy('launchd')" @click="toggleLaunchd">{{ launchd?.installed ? t('移除', 'Remove') : t('安装', 'Install') }}</a-button>
      </div>
    </div>
  </section>

  <section class="box">
    <div class="box-head"><h2>{{ t('数据', 'Data') }}</h2></div>
    <div class="field-row">
      <div><b>{{ t('数据目录', 'Data directory') }}</b><small>{{ t('账号库、设置、请求记录、用量账本与备份', 'Accounts, settings, request log, usage ledger and backups') }}</small></div>
      <div class="control"><code class="mono">{{ state.app?.dataDir }}</code><a-button size="small" @click="act('reveal', () => call('reveal_data_dir'), undefined, [])">{{ t('在 Finder 中显示', 'Show in Finder') }}</a-button></div>
    </div>
    <div class="field-row">
      <div><b>{{ t('导出 Qoder 账号备份', 'Export Qoder accounts') }}</b><small>{{ t('导出到「下载」目录（含备注与登录态）', 'Exports to Downloads (notes and logins included)') }}</small></div>
      <div class="control"><a-button :loading="busy('qexport')" @click="exportQoder">{{ t('导出', 'Export') }}</a-button></div>
    </div>
    <div class="field-row">
      <div><b>{{ t('导入 Qoder 账号备份', 'Import Qoder accounts') }}</b><small>{{ t('从 JSON 备份文件导入/更新', 'Import or update from a JSON backup') }}</small></div>
      <div class="control"><a-button @click="importPath = ''">{{ t('导入', 'Import') }}</a-button></div>
    </div>
    <div v-if="migration" class="field-row">
      <div><b>{{ t('旧应用数据迁移', 'Legacy data migration') }}</b><small>{{ t('首次启动时从原应用复制，原数据未改动', 'Copied once from the original apps; originals untouched') }}</small></div>
      <div class="control" style="flex-direction: column; align-items: flex-start; gap: 2px; font-size: 12px">
        <span v-if="migration.workbuddy">WorkBuddy API · {{ migration.workbuddy.files }} {{ t('个文件', 'files') }} · {{ migration.workbuddy.usageRows }} {{ t('条用量', 'usage rows') }}</span>
        <span v-if="migration.qoder">Qoder Switch · {{ migration.qoder.files }} {{ t('个文件', 'files') }} · {{ migration.qoder.usageRows }} {{ t('条用量', 'usage rows') }}</span>
        <span v-if="migration.trae">Trae Switch · {{ migration.trae.files }} {{ t('个文件', 'files') }} · {{ migration.trae.requests }} {{ t('条请求', 'requests') }}</span>
      </div>
    </div>
  </section>

  <a-modal :visible="importPath !== null" :title="t('导入 Qoder 账号备份', 'Import Qoder account backup')" :ok-loading="busy('qimport')" @ok="importQoder" @cancel="importPath = null">
    <a-input :model-value="importPath ?? ''" @update:model-value="(v: string) => (importPath = v)" :placeholder="t('备份 JSON 文件的完整路径，例如 ~/Downloads/qoder-accounts-backup.json', 'Full path of the backup JSON, e.g. ~/Downloads/qoder-accounts-backup.json')" />
  </a-modal>
  <section class="box">
    <div class="box-head"><h2>{{ t('运行事件（Trae）', 'Activity (Trae)') }}</h2></div>
    <div v-for="e in (state.pool?.trae?.events ?? []).slice(0, 30)" :key="e.id" class="kv">
      <span>{{ fmtDateTime(e.time) }} · {{ String(e.type).toUpperCase() }}</span><b>{{ e.message }}</b>
    </div>
    <a-empty v-if="!(state.pool?.trae?.events ?? []).length" :description="t('暂无事件', 'No events')" />
  </section>
</template>
