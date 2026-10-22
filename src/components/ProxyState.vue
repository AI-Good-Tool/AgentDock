<script setup lang="ts">
// Unified gateway switch + runtime state for one account (any provider).
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
  <div class="proxy-switch">
    <span>{{ t('反代', 'Proxy') }}</span>
    <a-switch size="small" :aria-label="t('参与反代', 'Use in proxy')" :model-value="rt?.enabled ?? true" :loading="state.pending.has(`enable:${key}`)" @change="(v) => toggle(!!v)" />
    <span v-if="rt?.authFailed" class="state-pill bad" @click="clear" style="cursor: pointer" :title="t('点击解除', 'Click to clear')">{{ t('鉴权失效', 'Auth failed') }}</span>
    <span v-else-if="cooling" class="state-pill warn" @click="clear" style="cursor: pointer" :title="t('点击解除', 'Click to clear')">{{ t('冷却', 'Cooldown') }} · {{ relative(rt?.cooldownUntil, state.now) }}</span>
    <span class="muted">{{ rt?.inFlight ?? 0 }} / {{ rt?.concurrency ?? 4 }} {{ t('并发', 'active') }}</span>
  </div>
</template>
