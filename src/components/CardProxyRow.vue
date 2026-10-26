<script setup lang="ts">
// WorkBuddy-style footer row for Qoder / Trae cards: proxy switch, runtime state, concurrency.
import { computed } from 'vue'
import { Message } from '@arco-design/web-vue'
import { t, relative } from '../i18n'
import { state, runtimeOf, act, refreshPool } from '../store'
import { call } from '../api'
const props = defineProps<{ provider: string; id: string }>()
const rt = computed(() => runtimeOf(props.provider, props.id))
const key = computed(() => `${props.provider}/${props.id}`)
const cooling = computed(() => (rt.value?.cooldownUntil ?? 0) > state.now)
function toggle(enabled: boolean) {
  return act(`enable:${key.value}`, () => call('set_account_enabled', { providerKey: props.provider, id: props.id, enabled }))
}
async function clear() {
  await call('clear_account_runtime', { key: key.value })
  await refreshPool()
  Message.success(t('已解除冷却/失效标记', 'Cooldown cleared'))
}
</script>
<template>
  <div class="account-toggle-row">
    <label
      :title="
        rt?.disabled
          ? t('账号已禁用，启用后才能开启反代', 'Enable the account to turn the proxy on')
          : t('停用后不再接收新请求，正在进行的请求会继续完成', 'Disabled accounts take no new requests; in-flight requests finish')
      "
      >{{ t('反代', 'Proxy') }}
      <a-switch
        size="small"
        :model-value="rt?.enabled ?? true"
        :disabled="rt?.disabled"
        :loading="state.pending.has(`enable:${key}`)"
        @change="(v) => toggle(!!v)"
    /></label>
    <slot />
    <span v-if="rt?.authFailed" class="auth-invalid-chip" :title="t('重新登录后请手动开启反代', 'Sign in again, then enable proxy manually')">{{ t('登录失效', 'Login invalid') }}</span>
    <button v-else-if="cooling" class="relogin-chip" :title="t('点击解除', 'Click to clear')" @click="clear">{{ t('冷却', 'Cooldown') }} · {{ relative(rt?.cooldownUntil, state.now) }}</button>
    <span>{{ rt?.inFlight ?? 0 }} / {{ rt?.concurrency ?? 4 }} {{ t('并发', 'active') }}</span>
  </div>
</template>
