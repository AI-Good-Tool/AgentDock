<script setup lang="ts">
// View / save one account's JSON, or import accounts from pasted JSON.
import { computed, onMounted, ref } from 'vue'
import { Message } from '@arco-design/web-vue'
import { IconCopy, IconDownload, IconEye, IconEyeInvisible } from '@arco-design/web-vue/es/icon'
import JsonView from './JsonView.vue'
import ProviderMark from './ProviderMark.vue'
import { call } from '../api'
import { t } from '../i18n'
import { act, busy, copy, providerLabel, refreshPool } from '../store'

const props = defineProps<{ mode: 'view' | 'import'; provider: string; id?: string; name?: string }>()
const emit = defineEmits<{ close: [] }>()

// view
const doc = ref<any>(null)
const error = ref('')
const reveal = ref(false)
onMounted(async () => {
  if (props.mode !== 'view') return
  try {
    doc.value = await call('account_json', { provider: props.provider, id: props.id })
  } catch (e) {
    error.value = String(e)
  }
})
const save = () => act('json-save', () => call('account_json_save', { provider: props.provider, id: props.id }), undefined, [])

// import
const text = ref('')
const region = ref<'cn' | 'intl'>('cn')
const parsed = computed<{ value?: any; error?: string; count?: number }>(() => {
  const raw = text.value.trim()
  if (!raw) return {}
  try {
    const value = JSON.parse(raw)
    const list = Array.isArray(value) ? value : Array.isArray(value?.accounts) ? value.accounts : [value]
    return { value, count: list.length }
  } catch (e) {
    return { error: t('JSON 格式错误：', 'Invalid JSON: ') + (e instanceof Error ? e.message : String(e)) }
  }
})
function format() {
  if (parsed.value.value !== undefined) text.value = JSON.stringify(parsed.value.value, null, 2)
}
async function doImport() {
  if (parsed.value.value === undefined) {
    Message.error(parsed.value.error || t('请粘贴账号 JSON', 'Paste account JSON'))
    return false
  }
  const r = await act('json-import', () => call('account_json_import', { provider: props.provider, region: region.value, json: parsed.value.value }), undefined, [refreshPool])
  if (r) emit('close')
  return !!r
}
const hints: Record<string, [string, string]> = {
  workbuddy: ['WorkBuddy Switch 账号记录：需要 uid 与 access_token（可选 nickname、domain、variant）。会写入 ~/.wb-switch 账号库并加入号池。', 'WorkBuddy Switch record: needs uid and access_token (optional nickname, domain, variant). Saved to the ~/.wb-switch library and added to the pool.'],
  qoder: ['Qoder Switch 账号：需要 edition 与 uid，以及 token / refreshToken 等登录信息；同一账号会被更新。', 'Qoder Switch account: needs edition and uid plus token / refreshToken; an existing account is updated.'],
  trae: ['Trae 账号：需要 user.userId、user.token、user.expiredAt 与 user.account；同一账号会被更新。', 'Trae account: needs user.userId, user.token, user.expiredAt and user.account; an existing account is updated.'],
}
</script>

<template>
  <a-modal
    visible
    :width="760"
    :title="mode === 'view' ? t('账号 JSON', 'Account JSON') : t('导入账号 JSON', 'Import account JSON')"
    :footer="mode === 'import'"
    :ok-text="t('导入', 'Import')"
    :ok-loading="busy('json-import')"
    :on-before-ok="doImport"
    @cancel="emit('close')"
  >
    <template v-if="mode === 'view'">
      <div class="json-head">
        <span class="platform-opt"><ProviderMark :provider="provider" />{{ providerLabel(provider) }}<template v-if="name"> · {{ name }}</template></span>
        <span class="grow"></span>
        <a-button size="small" @click="reveal = !reveal"><template #icon><IconEye v-if="!reveal" /><IconEyeInvisible v-else /></template>{{ reveal ? t('隐藏凭据', 'Hide secrets') : t('显示凭据', 'Show secrets') }}</a-button>
        <a-button size="small" :disabled="!doc" @click="copy(JSON.stringify(doc, null, 2))"><template #icon><IconCopy /></template>{{ t('复制完整 JSON', 'Copy full JSON') }}</a-button>
        <a-button size="small" type="primary" :disabled="!doc" :loading="busy('json-save')" @click="save"><template #icon><IconDownload /></template>{{ t('导出到「下载」', 'Save to Downloads') }}</a-button>
      </div>
      <a-alert v-if="error" type="error">{{ error }}</a-alert>
      <a-spin v-else-if="!doc" style="display: block; padding: 40px 0; text-align: center" />
      <JsonView v-else :value="doc" :mask="!reveal" />
      <p class="hint">{{ t('复制与导出的都是完整 JSON（含登录凭据），可在「添加账号 → 导入账号 JSON」或原工具中导入；请妥善保管。', 'Copy and export contain the full JSON with login credentials; import it via Add account → Import account JSON or the original tool. Keep it private.') }}</p>
    </template>

    <template v-else>
      <div class="json-head">
        <span class="platform-opt"><ProviderMark :provider="provider" />{{ providerLabel(provider) }}</span>
        <span class="grow"></span>
        <template v-if="provider !== 'qoder'">
          <span class="muted">{{ t('未注明版本时导入为', 'Default version') }}</span>
          <a-radio-group v-model="region" type="button" size="small">
            <a-radio value="cn">{{ t('国内版', 'China') }}</a-radio>
            <a-radio value="intl">{{ t('国际版', 'International') }}</a-radio>
          </a-radio-group>
        </template>
        <a-button size="small" :disabled="parsed.value === undefined" @click="format">{{ t('格式化', 'Format') }}</a-button>
      </div>
      <a-textarea v-model="text" class="json-input mono" :auto-size="{ minRows: 12, maxRows: 22 }" :placeholder="t('粘贴单个账号、账号数组，或从 AgentDock 导出的账号 JSON', 'Paste one account, an array, or an account JSON exported by AgentDock')" />
      <p v-if="parsed.error" class="error-text json-status">{{ parsed.error }}</p>
      <p v-else-if="parsed.count" class="json-status muted">{{ t(`识别到 ${parsed.count} 个账号`, `${parsed.count} account(s) detected`) }}</p>
      <p class="hint">{{ t(...hints[provider]) }}</p>
    </template>
  </a-modal>
</template>

<style>
.json-head {
  display: flex;
  align-items: center;
  gap: 8px;
  flex-wrap: wrap;
  margin-bottom: 12px;
}
.json-head .grow {
  flex: 1;
}
.json-input textarea {
  font-family: 'SF Mono', ui-monospace, Menlo, monospace;
  font-size: 12px;
  line-height: 1.6;
}
.json-status {
  margin: 6px 0 0;
  font-size: 12px;
}
</style>
