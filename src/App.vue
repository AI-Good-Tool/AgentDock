<script setup lang="ts">
import ProviderMark from './components/ProviderMark.vue'
import BrandMark from './components/BrandMark.vue'
import { computed, onMounted, watch } from 'vue'
import { Message } from '@arco-design/web-vue'
import {
  IconApps,
  IconUserGroup,
  IconStorage,
  IconThunderbolt,
  IconCalendar,
  IconSettings,
  IconCode,
  IconBarChart,
  IconMessage,
  IconLink,
  IconPoweroff,
  IconPlayArrow,
  IconFile,
  IconInfoCircle,
} from '@arco-design/web-vue/es/icon'
import { t, isZh } from './i18n'
import { state, page, startPolling, act, refreshApp } from './store'
import { call, desktop, previewMode } from './api'
import Overview from './pages/Overview.vue'
import Accounts from './pages/Accounts.vue'
import Models from './pages/Models.vue'
import Requests from './pages/Requests.vue'
import Checkin from './pages/Checkin.vue'
import Credits from './pages/Credits.vue'
import Tokens from './pages/Tokens.vue'
import Sessions from './pages/Sessions.vue'
import Access from './pages/Access.vue'
import Preferences from './pages/Preferences.vue'
import Logs from './pages/Logs.vue'
import About from './pages/About.vue'

const nav = computed(() => [
  { id: 'overview', label: t('工作台', 'Overview'), icon: IconApps },
  {
    id: 'accounts',
    label: t('账号池', 'Accounts'),
    icon: IconUserGroup,
    count: accountCount.value,
  },
  {
    id: 'models',
    label: t('模型目录', 'Models'),
    icon: IconStorage,
    count: state.catalog?.routes.length,
  },
  { id: 'requests', label: t('请求记录', 'Requests'), icon: IconThunderbolt },
  { id: 'checkin', label: t('签到与领取', 'Daily Rewards'), icon: IconCalendar },
  { id: 'credits', label: t('积分统计', 'Credits'), icon: IconBarChart },
  { id: 'tokens', label: t('Token 统计', 'Token Usage'), icon: IconCode },
  { id: 'sessions', label: t('会话管理', 'Sessions'), icon: IconMessage },
  { id: 'access', label: t('接入设置', 'API Access'), icon: IconLink },
  ...(logging.value ? [{ id: 'logs', label: t('运行日志', 'Runtime Log'), icon: IconFile }] : []),
  { id: 'settings', label: t('偏好设置', 'Preferences'), icon: IconSettings },
  { id: 'about', label: t('关于', 'About'), icon: IconInfoCircle },
])
const logging = computed(() => state.app?.settings.logging ?? true)
// Leave the Logs page when its menu is switched off.
watch(logging, (on) => {
  if (!on && page.value === 'logs') page.value = 'settings'
})
const accountCount = computed(() => {
  const p = state.pool
  if (!p) return undefined
  return (
    (p.workbuddy?.accounts?.length ?? 0) +
    (p.qoder?.accounts?.length ?? 0) +
    (p.trae?.accounts?.length ?? 0)
  )
})
const pages: Record<string, any> = {
  overview: Overview,
  accounts: Accounts,
  models: Models,
  requests: Requests,
  checkin: Checkin,
  credits: Credits,
  tokens: Tokens,
  sessions: Sessions,
  access: Access,
  settings: Preferences,
  logs: Logs,
  about: About,
}
const running = computed(() => !!state.app?.running)
const lanHost = computed(() => (running.value && state.app?.lan ? state.app.lanAddresses?.[0] : undefined))
const lanTitle = computed(() => {
  if (!running.value) return ''
  if (!state.app?.lan) return t('仅本机可访问', 'This computer only')
  const all = (state.app.lanAddresses ?? []).map((ip) => `${ip}:${state.app?.port}`)
  return all.length
    ? t('局域网设备可通过 ', 'LAN devices can use ') + all.join(t('、', ', ')) + t(' 访问（需 API Key）', ' (API key required)')
    : t('已允许局域网访问，但未检测到局域网地址', 'LAN access is on, but no LAN address was found')
})
function toggle() {
  return act(
    'gateway',
    () => call('gateway_toggle', { enabled: !running.value }),
    running.value
      ? t('服务正在停止，进行中的请求完成后关闭', 'Stopping after active requests finish')
      : t('本地网关已启动', 'Local gateway started'),
    [refreshApp]
  )
}
onMounted(async () => {
  startPolling()
  if (!desktop) return
  // After an in-app update: confirm the new version started, then surface any install error.
  try {
    await call('app_update_confirm_restart')
    const installError = await call<string | null>('app_update_take_install_error')
    if (installError) Message.error({ content: installError, duration: 10000 })
  } catch (e) {
    Message.error(String(e))
  }
})
</script>

<template>
  <div class="app-shell" :class="{ en: !isZh }">
    <aside class="sidebar">
      <div class="brand">
        <BrandMark />
        <div>
          <strong>Agent<b>Dock</b></strong
          ><small>{{ t('统一模型网关', 'UNIFIED MODEL GATEWAY') }}</small>
        </div>
      </div>
      <div class="space-label">{{ t('工作空间', 'Workspace') }} <span>LOCAL</span></div>
      <nav>
        <button
          v-for="item in nav"
          :key="item.id"
          :class="{ active: page === item.id }"
          @click="page = item.id"
        >
          <component :is="item.icon" /><span>{{ item.label }}</span
          ><span v-if="item.count" class="nav-count">{{ item.count }}</span>
        </button>
      </nav>
      <div class="sidebar-providers">
        <ProviderMark provider="workbuddy" /><ProviderMark provider="qoder" /><ProviderMark
          provider="trae"
        />
        <small>WorkBuddy · Qoder · Trae</small>
      </div>
      <div class="sidebar-footer">
        <span class="dot" :class="{ online: running }"></span
        >{{ running ? t('网关运行中', 'Gateway running') : t('网关未启动', 'Gateway stopped') }}
        <small>v{{ state.app?.version ?? '1.0.4' }}</small>
      </div>
    </aside>
    <div class="main-shell">
      <header class="topbar">
        <div>AgentDock <span>/</span> {{ nav.find((n) => n.id === page)?.label }}</div>
        <div class="topbar-right">
          <span class="gateway-chip" :class="{ online: running }" :title="lanTitle">
            <i class="dot" :class="{ online: running }"></i>
            <span class="mono">{{ running ? `127.0.0.1:${state.app?.port}` : t('未启动', 'Stopped') }}</span>
            <template v-if="lanHost">
              <span class="chip-sep">·</span>
              <span class="mono">{{ lanHost }}:{{ state.app?.port }}</span>
            </template>
          </span>
          <a-button
            class="gateway-btn"
            :type="running ? 'outline' : 'primary'"
            :status="running ? 'danger' : 'normal'"
            :loading="state.pending.has('gateway')"
            @click="toggle"
          >
            <template #icon><IconPoweroff v-if="running" /><IconPlayArrow v-else /></template>
            {{ running ? t('停止服务', 'Stop') : t('启动服务', 'Start') }}
          </a-button>
        </div>
      </header>
      <main>
        <div v-if="!desktop && !previewMode" class="preview-banner">
          {{
            t(
              '当前为浏览器预览，账号与网关操作请在 AgentDock 桌面应用中使用。',
              'Browser preview: use the AgentDock desktop app for accounts and the gateway.'
            )
          }}
        </div>
        <div v-if="state.app?.error" class="notice warn-notice">{{ state.app.error }}</div>
        <component :is="pages[page]" :key="page" />
      </main>
    </div>
  </div>
</template>
