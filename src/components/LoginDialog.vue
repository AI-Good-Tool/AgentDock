<script setup lang="ts">
// Browser sign-in for Qoder (OAuth device flow) and Trae (PKCE + loopback).
import { ref, onUnmounted, onMounted } from 'vue'
import { call } from '../api'
import { t } from '../i18n'
import { refreshPool, copy } from '../store'
const props = defineProps<{ provider: 'qoder' | 'trae'; region: 'cn' | 'intl' }>()
const emit = defineEmits<{ close: [] }>()
const url = ref('')
const status = ref<'starting' | 'waiting' | 'done' | 'error'>('starting')
const message = ref('')
let timer: ReturnType<typeof setTimeout> | undefined
let alive = true
const started = Date.now()
onUnmounted(() => {
  alive = false
  clearTimeout(timer)
})
async function pollQoder(attempt: any) {
  if (!alive) return
  if (Date.now() - started > 5 * 60e3) {
    status.value = 'error'
    message.value = t('等待超时，请重新发起登录', 'Timed out; start again')
    return
  }
  try {
    const r = await call('qoder_login_poll', { attempt })
    if (r.done) {
      status.value = 'done'
      message.value = r.message
      await refreshPool()
      return
    }
  } catch (e) {
    status.value = 'error'
    message.value = String(e)
    return
  }
  timer = setTimeout(() => pollQoder(attempt), 2500)
}
async function pollTrae() {
  if (!alive) return
  const s = await call('trae_state').catch(() => null)
  const rec = s?.logins?.find((r: any) => r.variant === props.region)
  if (rec?.status === 'success') {
    status.value = 'done'
    message.value = t('账号已添加', 'Account added')
    await refreshPool()
    return
  }
  if (rec?.status === 'error' || rec?.status === 'expired') {
    status.value = 'error'
    message.value = rec.error || t('登录已过期，请重新发起', 'Login expired; start again')
    return
  }
  timer = setTimeout(pollTrae, 2000)
}
onMounted(async () => {
  try {
    if (props.provider === 'qoder') {
      const r = await call('qoder_login_begin', { edition: props.region })
      url.value = r.url
      await call('open_url', { url: r.url }).catch(() => {})
      status.value = 'waiting'
      timer = setTimeout(() => pollQoder(r.attempt), 1500)
    } else {
      await call('trae_login', { variant: props.region })
      status.value = 'waiting'
      timer = setTimeout(pollTrae, 2000)
    }
  } catch (e) {
    status.value = 'error'
    message.value = String(e)
  }
})
const name = (props.provider === 'qoder' ? 'Qoder' : 'Trae') + (props.region === 'cn' ? (props.provider === 'qoder' ? ' CN' : ' CN') : '')
</script>
<template>
  <a-modal :visible="true" :title="t(`添加 ${name} 账号`, `Add ${name} account`)" :footer="false" @cancel="emit('close')">
    <p>{{ t(`将打开系统浏览器进入 ${name} 官方登录页，完成登录后凭证会自动保存到本机。`, `Your browser opens the official ${name} sign-in page; the credentials are saved locally when you finish.`) }}</p>
    <template v-if="url">
      <p class="hint">{{ t('如浏览器未打开，可复制链接：', 'If the browser did not open, copy the link:') }}</p>
      <p class="mono" style="overflow-wrap: anywhere; font-size: 12px; user-select: text">{{ url }}</p>
      <a-button size="small" @click="copy(url)">{{ t('复制链接', 'Copy link') }}</a-button>
    </template>
    <a-alert v-if="status === 'starting'" style="margin-top: 14px">{{ t('正在发起登录…', 'Starting…') }}</a-alert>
    <a-alert v-if="status === 'waiting'" style="margin-top: 14px">{{ t('等待浏览器登录授权…（最多 5 分钟）', 'Waiting for browser authorization… (up to 5 minutes)') }}</a-alert>
    <a-alert v-if="status === 'done'" type="success" style="margin-top: 14px">{{ message }}</a-alert>
    <a-alert v-if="status === 'error'" type="error" style="margin-top: 14px">{{ message }}</a-alert>
    <div style="margin-top: 18px; text-align: right">
      <a-button type="primary" @click="emit('close')">{{ status === 'done' ? t('完成', 'Done') : t('关闭', 'Close') }}</a-button>
    </div>
  </a-modal>
</template>
