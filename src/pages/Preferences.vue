<script setup lang="ts">
import { computed, ref, onMounted } from 'vue'
import { Message, Modal } from '@arco-design/web-vue'
import { IconDownload, IconUpload, IconFolder, IconFile } from '@arco-design/web-vue/es/icon'
import PageHead from '../components/PageHead.vue'
import { call, isWindows } from '../api'
import ProviderMark from '../components/ProviderMark.vue'
import { t, lang, systemLang, systemZone, zoneLabel, zoneOptions, defaultZone, zone, followSystemZone, fmtDateTime, langSetting, tzSetting } from '../i18n'
import { state, act, busy, saveSettings, refreshPool, themeSetting, page } from '../store'

onMounted(refreshPool)
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
// All-platform backup
type Provider = 'workbuddy' | 'qoder' | 'trae'
const platforms: { key: Provider; label: string }[] = [
  { key: 'workbuddy', label: 'WorkBuddy' },
  { key: 'qoder', label: 'Qoder' },
  { key: 'trae', label: 'Trae' },
]
const exportParts = ref({ providers: ['workbuddy', 'qoder', 'trae'] as Provider[], accounts: true, settings: true })
const lastExport = ref('')
async function exportBundle() {
  const p = exportParts.value
  if (!p.accounts && !p.settings) return Message.error(t('请选择账号或设置', 'Choose accounts or settings'))
  if (!p.providers.length && !p.settings) return Message.error(t('请至少选择一个平台', 'Choose at least one platform'))
  const r = await act('backup-export', () => call('backup_export', { parts: p }))
  if (r?.path) {
    lastExport.value = r.path
    Message.success(t(`已导出到 ${r.path}`, `Exported to ${r.path}`))
  }
}
const importPath = ref('')
const inspected = ref<{ path: string; summary: any } | null>(null)
const importParts = ref({ providers: [] as Provider[], accounts: true, settings: true })
async function inspectBundle() {
  const path = importPath.value.trim()
  if (!path) return Message.error(t('请输入备份文件路径', 'Enter the backup file path'))
  const r = await act('backup-inspect', () => call('backup_inspect', { path }))
  if (!r) return
  inspected.value = r
  importParts.value.providers = platforms.filter((p) => r.summary?.[p.key]?.present).map((p) => p.key)
}
function importBundle() {
  const info = inspected.value
  if (!info) return
  Modal.confirm({
    title: t('导入备份', 'Import backup'),
    content: t(
      '账号按平台身份合并（同一账号以备份为准），不会删除现有账号；勾选「设置」时会覆盖对应设置。',
      'Accounts merge by platform identity (the backup wins for matching accounts); nothing is deleted. With “Settings” checked, the matching settings are replaced.'
    ),
    onOk: async () => {
      const r = await act('backup-import', () => call('backup_import', { path: info.path, parts: importParts.value }), t('备份已导入', 'Backup imported'), [refreshPool])
      if (r) inspected.value = null
    },
  })
}
function summaryText(s: any, p: Provider) {
  const v = s?.[p]
  if (!v?.present) return t('不含', 'Not included')
  const parts: string[] = []
  if (v.accounts != null) parts.push(t(`${v.accounts} 个账号`, `${v.accounts} accounts`))
  if (v.settings) parts.push(t('设置', 'settings'))
  return parts.join(' · ') || '—'
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
    <p class="hint">{{ t('开启后跟随系统语言：系统为中文时显示中文，其他任何语言一律显示英文；关闭后使用选择的语言。', 'When on, follows the system language: Chinese systems show Chinese, every other language shows English. When off, the selected language is used.') }}</p>
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
      {{ t('开启后跟随系统时区（出差自动切换）；关闭后使用选择的时区，默认中文为北京时间（上海时区）、英文为美国东部（纽约）时区。界面时间、请求记录与 Token 统计的日期分组均按此时区计算（自动处理夏令时）。', 'When on, follows the system time zone (switches automatically when you travel). When off, the selected zone is used — Beijing (Shanghai) by default for Chinese, US Eastern (New York) for English. Display times, request logs and token-statistics day boundaries use this zone (DST aware).') }}
      <br />{{ t('当前时间：', 'Now: ') }}{{ fmtDateTime(state.now) }}
    </p>
    <p class="hint">{{ t('官方签到日界线、WorkBuddy 官方积分统计仍按上海时间计算，与官方口径一致。', 'Official check-in days and WorkBuddy official credit statistics still use Shanghai time, matching the official definition.') }}</p>
  </section>

  <section class="box">
    <div class="box-head"><h2>{{ t('运行日志', 'Runtime log') }}</h2></div>
    <div class="field-row" style="border-bottom: none">
      <div>
        <b>{{ t('记录运行日志', 'Record runtime log') }}</b>
        <small>{{ t('开启后显示「运行日志」菜单，记录网关请求、WorkBuddy 设备 SDK / Agent 调用、签到与备份；关闭后不产生开销。', 'Shows the Runtime Log menu and records gateway requests, WorkBuddy device SDK / Agent calls, check-ins and backups. No overhead when off.') }}</small>
      </div>
      <div class="control end">
        <a-button v-if="state.app?.settings.logging" size="small" @click="page = 'logs'"><template #icon><IconFile /></template>{{ t('查看日志', 'Open log') }}</a-button>
        <a-switch :model-value="state.app?.settings.logging ?? true" @change="(v) => saveSettings({ logging: !!v })" />
      </div>
    </div>
  </section>

  <section class="box">
    <div class="box-head">
      <div>
        <h2>{{ t('数据与备份', 'Data & backup') }}</h2>
        <p>{{ t('WorkBuddy、Qoder、Trae 的账号与设置，连同 AgentDock 的网关、模型别名与签到设置，一个文件导出 / 导入。', 'Accounts and settings of WorkBuddy, Qoder and Trae, plus AgentDock gateway, model aliases and check-in settings, in one file.') }}</p>
      </div>
    </div>
    <div class="field-row">
      <div>
        <b>{{ t('数据目录', 'Data directory') }}</b>
        <small>{{ t('账号库、设置、请求记录、用量账本与日志', 'Accounts, settings, request log, usage ledger and logs') }}</small>
      </div>
      <div class="control end">
        <code class="path-chip mono">{{ state.app?.dataDir }}</code>
        <a-button size="small" @click="act('reveal', () => call('reveal_data_dir'), undefined, [])"><template #icon><IconFolder /></template>{{ isWindows ? t('在资源管理器中显示', 'Show in Explorer') : t('在 Finder 中显示', 'Show in Finder') }}</a-button>
      </div>
    </div>

    <div class="backup-grid">
      <div class="backup-card">
        <div class="backup-title"><IconDownload /><b>{{ t('导出备份', 'Export') }}</b></div>
        <div class="backup-line">
          <span class="backup-label">{{ t('平台', 'Platforms') }}</span>
          <a-checkbox-group v-model="exportParts.providers">
            <a-checkbox v-for="p in platforms" :key="p.key" :value="p.key"><span class="platform-opt"><ProviderMark :provider="p.key" />{{ p.label }}</span></a-checkbox>
          </a-checkbox-group>
        </div>
        <div class="backup-line">
          <span class="backup-label">{{ t('内容', 'Content') }}</span>
          <a-checkbox v-model="exportParts.accounts">{{ t('账号（含登录态）', 'Accounts (with logins)') }}</a-checkbox>
          <a-checkbox v-model="exportParts.settings">{{ t('设置', 'Settings') }}</a-checkbox>
        </div>
        <p class="backup-note warn">{{ t('备份包含可直接使用的登录令牌，保存到「下载」目录且仅本人可读，请勿外传。', 'The file contains usable login tokens. It is saved to Downloads, readable only by you; do not share it.') }}</p>
        <div class="backup-actions">
          <span v-if="lastExport" class="muted mono ellipsis" :title="lastExport">{{ lastExport }}</span>
          <a-button type="primary" :loading="busy('backup-export')" @click="exportBundle"><template #icon><IconDownload /></template>{{ t('导出到「下载」', 'Export to Downloads') }}</a-button>
        </div>
      </div>

      <div class="backup-card">
        <div class="backup-title"><IconUpload /><b>{{ t('导入备份', 'Import') }}</b></div>
        <div class="backup-line nowrap">
          <a-input v-model="importPath" :placeholder="t('备份文件路径，如 ~/Downloads/agentdock-backup-….json', 'Backup path, e.g. ~/Downloads/agentdock-backup-….json')" allow-clear @press-enter="inspectBundle" @input="inspected = null" @clear="inspected = null" />
          <a-button :loading="busy('backup-inspect')" @click="inspectBundle">{{ t('读取', 'Read') }}</a-button>
        </div>
        <template v-if="inspected">
          <div class="backup-summary">
            <div v-for="p in platforms" :key="p.key" class="backup-summary-row">
              <a-checkbox v-model="importParts.providers" :value="p.key" :disabled="!inspected.summary?.[p.key]?.present"><span class="platform-opt"><ProviderMark :provider="p.key" />{{ p.label }}</span></a-checkbox>
              <span class="muted">{{ summaryText(inspected.summary, p.key) }}</span>
            </div>
          </div>
          <div class="backup-line">
            <span class="backup-label">{{ t('内容', 'Content') }}</span>
            <a-checkbox v-model="importParts.accounts">{{ t('账号', 'Accounts') }}</a-checkbox>
            <a-checkbox v-model="importParts.settings">{{ t('设置', 'Settings') }}{{ inspected.summary?.agentdockSettings ? t('（含 AgentDock）', ' (incl. AgentDock)') : '' }}</a-checkbox>
          </div>
          <p class="backup-note">{{ inspected.summary?.exportedAt ? t('导出于 ', 'Exported ') + fmtDateTime(inspected.summary.exportedAt) + ' · ' : '' }}{{ t('按账号合并，不删除现有账号。', 'Merges by account; deletes nothing.') }}</p>
          <div class="backup-actions">
            <a-button type="primary" :disabled="!importParts.providers.length && !importParts.settings" :loading="busy('backup-import')" @click="importBundle"><template #icon><IconUpload /></template>{{ t('导入所选内容', 'Import selected') }}</a-button>
          </div>
        </template>
        <p v-else class="backup-note">{{ t('先读取文件，确认内容后再选择要导入的平台。也支持 Qoder Switch 旧版账号备份。', 'Read the file first, then choose what to import. Legacy Qoder Switch backups also work.') }}</p>
      </div>
    </div>

    <div v-if="migration" class="field-row" style="border-bottom: none">
      <div>
        <b>{{ t('旧应用数据迁移', 'Legacy data migration') }}</b>
        <small>{{ t('首次启动时从原应用复制，原数据未改动', 'Copied once from the original apps; originals untouched') }}</small>
      </div>
      <div class="migration-list">
        <template v-if="migration.workbuddy"><span class="platform-opt"><ProviderMark provider="workbuddy" />WorkBuddy API</span><span>{{ migration.workbuddy.files }} {{ t('个文件', 'files') }}</span><span>{{ migration.workbuddy.usageRows }} {{ t('条用量', 'usage rows') }}</span></template>
        <template v-if="migration.qoder"><span class="platform-opt"><ProviderMark provider="qoder" />Qoder Switch</span><span>{{ migration.qoder.files }} {{ t('个文件', 'files') }}</span><span>{{ migration.qoder.usageRows }} {{ t('条用量', 'usage rows') }}</span></template>
        <template v-if="migration.trae"><span class="platform-opt"><ProviderMark provider="trae" />Trae Switch</span><span>{{ migration.trae.files }} {{ t('个文件', 'files') }}</span><span>{{ migration.trae.requests }} {{ t('条请求', 'requests') }}</span></template>
      </div>
    </div>
  </section>

  <section class="box">
    <div class="box-head"><h2>{{ t('运行事件（Trae）', 'Activity (Trae)') }}</h2></div>
    <div v-for="e in (state.pool?.trae?.events ?? []).slice(0, 30)" :key="e.id" class="kv">
      <span>{{ fmtDateTime(e.time) }} · {{ String(e.type).toUpperCase() }}</span><b>{{ e.message }}</b>
    </div>
    <a-empty v-if="!(state.pool?.trae?.events ?? []).length" :description="t('暂无事件', 'No events')" />
  </section>
</template>
