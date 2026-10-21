<script setup lang="ts">
// Trae: all credit packs, or import a full account JSON.
import { ref, computed } from 'vue'
import { Message } from '@arco-design/web-vue'
import { call } from '../api'
import { t, fmtNumber, fmtDateTime } from '../i18n'
import { refreshPool, state } from '../store'
const props = defineProps<{ mode: 'packs' | 'import'; account?: any; region?: 'cn' | 'intl' }>()
const emit = defineEmits<{ close: [] }>()
const variant = ref<'cn' | 'intl'>(props.region ?? 'cn')
const text = ref('')
const saving = ref(false)
const packs = computed(() => props.account?.creditPacks || [])
async function save() {
  let account
  try {
    account = JSON.parse(text.value)
  } catch {
    Message.error(t('请输入有效的账号 JSON', 'Enter valid account JSON'))
    return
  }
  saving.value = true
  try {
    await call('trae_import', { variant: variant.value, account })
    Message.success(t('账号已导入', 'Account imported'))
    await refreshPool()
    emit('close')
  } catch (e) {
    Message.error(String(e))
  } finally {
    saving.value = false
  }
}
</script>
<template>
  <a-modal v-if="mode === 'packs'" :visible="true" :title="(account?.note || account?.username) + t(' · 全部积分包', ' · All credit packs')" :footer="false" :width="620" @cancel="emit('close')">
    <div v-for="(p, i) in packs" :key="i" style="margin-bottom: 12px">
      <div class="pack-row">
        <b>{{ fmtNumber(p.remaining) }} / {{ fmtNumber(p.total) }}</b><span>{{ p.name }}</span><time :class="{ near: p.expiresAt && Date.parse(p.expiresAt) - state.now < 7 * 864e5 }">{{ p.expiresAt ? fmtDateTime(p.expiresAt) : t('未提供到期时间', 'No expiry') }}</time>
      </div>
      <div class="track" style="margin-top: 5px"><i :class="{ near: !p.active }" :style="{ width: `${Math.max(0, Math.min(100, (p.remaining / p.total) * 100))}%` }"></i></div>
      <small v-if="!p.active" class="muted">{{ t('已过期或已用完', 'Expired or used up') }}</small>
    </div>
    <a-empty v-if="!packs.length" :description="t('暂无积分包，请先刷新用量', 'No packs; refresh usage first')" />
  </a-modal>
  <a-modal v-else :visible="true" :title="t('导入 Trae 账号 JSON', 'Import Trae account JSON')" :ok-loading="saving" @ok="save" @cancel="emit('close')">
    <a-radio-group v-model="variant" type="button" style="margin-bottom: 12px">
      <a-radio value="cn">Trae CN</a-radio>
      <a-radio value="intl">Trae</a-radio>
    </a-radio-group>
    <a-textarea v-model="text" :auto-size="{ minRows: 8, maxRows: 14 }" :placeholder="t('粘贴包含 userId、token、expiredAt、account 的完整账号 JSON', 'Paste the full account JSON with userId, token, expiredAt and account')" />
    <p class="hint">{{ t('凭证仅保存在本机加密账号库中。', 'Credentials are kept only in the local encrypted vault.') }}</p>
  </a-modal>
</template>
