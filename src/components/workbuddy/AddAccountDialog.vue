<script setup lang="ts">
import { ref, onUnmounted } from 'vue'
import { Message, Modal as AModal, Space as ASpace } from '@arco-design/web-vue'
import { call } from '../../wb-api'
import { t } from '../../i18n'
const emit = defineEmits<{ added: []; close: [] }>()
const props = defineProps<{ initialVariant?: string; expectedAccountId?: string }>()
const variant = ref(props.initialVariant === 'ai' ? 'ai' : 'cn')
const busy = ref(false),
  waiting = ref(false),
  finished = ref(false)
const uri = ref(''),
  error = ref('')
let alive = true
let timer: ReturnType<typeof setTimeout> | undefined
onUnmounted(() => {
  alive = false
  clearTimeout(timer)
})
async function openLink() {
  try {
    await call('wb_open_oauth_link', { url: uri.value })
  } catch {
    if (alive) error.value = t('打开浏览器失败，请复制链接后手动打开', 'Could not open the browser; copy the link')
  }
}
async function copyLink() {
  try {
    await navigator.clipboard.writeText(uri.value)
    Message.success(t('授权链接已复制', 'Link copied'))
  } catch {
    Message.error(t('复制失败，请手动选中链接复制', 'Copy failed; select the link manually'))
  }
}
async function poll(loginId: string) {
  if (!alive) return
  try {
    const result = await call<{ done: boolean; error?: string; added?: boolean; accountId?: string }>('wb_oauth_poll', {
      loginId,
    })
    if (!alive) return
    if (result.done) {
      waiting.value = false
      if (result.added) {
        emit('added')
        if (props.expectedAccountId && result.accountId !== props.expectedAccountId) {
          error.value = t('授权的是另一个账号，已添加到账号池；原账号仍需重新登录。请使用原账号再次授权。', 'A different account was added. The original still needs sign-in; authorize the original account again.')
          return
        }
        finished.value = true
      } else error.value = result.error || t('登录失败，请重新发起', 'Login failed; try again')
      return
    }
    timer = setTimeout(() => void poll(loginId), 1500)
  } catch {
    if (alive) {
      waiting.value = false
      error.value = t('账号接入未完成，请检查 Switch 账号库或重新登录', 'Account not added; check the Switch store or sign in again')
    }
  }
}
async function start() {
  if (busy.value || waiting.value) return
  busy.value = true
  error.value = ''
  try {
    const result = await call<{ loginId: string; verificationUri: string }>('wb_oauth_start', {
      variant: variant.value,
      confirmed: true,
    })
    if (!alive) return
    uri.value = result.verificationUri
    waiting.value = true
    if (variant.value === 'cn') await openLink()
    void poll(result.loginId)
  } catch {
    if (alive) error.value = t('无法发起登录，请检查网络后重试', 'Could not start login; check your network')
  } finally {
    if (alive) busy.value = false
  }
}
</script>
<template>
  <a-modal :visible="true" :title="expectedAccountId ? t('重新登录 WorkBuddy', 'Sign in to WorkBuddy again') : t('添加 WorkBuddy 账号', 'Add WorkBuddy account')" :footer="false" @cancel="emit('close')">
    <p>
      {{ t('通过官方 OAuth 授权，凭据仅保存到本机 Switch 账号库；成功后将本次账号接入反代池，不切换原客户端登录。', 'Authorize via official OAuth. Credentials stay in the local Switch store; the account joins the proxy pool without changing your client login.') }}
    </p>
    <a-radio-group v-model="variant" type="button" :disabled="busy || waiting || finished">
      <a-radio value="cn">{{ t('国内版 · 扫码授权', 'China · QR code') }}</a-radio>
      <a-radio value="ai">{{ t('国际版 · Web 登录', 'International · Web') }}</a-radio>
    </a-radio-group>
    <p v-if="variant === 'ai'">{{ t('国际版请复制授权链接到浏览器无痕窗口，避免复用已登录账号。', 'Open the link in a private window to avoid reusing a signed-in account.') }}</p>
    <p v-if="uri" style="overflow-wrap: anywhere; user-select: text">{{ uri }}</p>
    <a-space v-if="uri && !finished">
      <a-button @click="copyLink">{{ t('复制链接', 'Copy link') }}</a-button>
      <a-button @click="openLink">{{ t('在浏览器打开', 'Open in browser') }}</a-button>
    </a-space>
    <p v-if="waiting">
      {{ t('正在等待官方授权…请勿重复提交。关闭窗口会停止后续轮询，已完成的账号保存不会撤销。', 'Waiting for authorization… Closing stops polling; a saved account is kept.') }}
    </p>
    <a-alert v-if="error" type="error">{{ error }}</a-alert>
    <a-alert v-if="finished" type="success">{{ expectedAccountId ? t('授权已完成，登录状态已更新，积分正在刷新；反代开关保持原状，可按需手动开启。', 'Sign-in updated; credits are refreshing. The proxy switch is unchanged; enable it manually when needed.') : t('账号已接入，原有账号开关与设置保持不变。', 'Account added; existing settings are unchanged.') }}</a-alert>
    <div style="margin-top: 20px">
      <a-button v-if="!finished" type="primary" :loading="busy" :disabled="waiting" @click="start"
        >{{ t('同意保存账号并开始授权', 'Agree and start authorization') }}</a-button
      >
      <a-button v-else type="primary" @click="emit('close')">{{ t('完成', 'Done') }}</a-button>
    </div>
  </a-modal>
</template>
