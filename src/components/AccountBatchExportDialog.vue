<script setup lang="ts">
import { computed, ref } from 'vue'
import { IconDownload } from '@arco-design/web-vue/es/icon'
import ProviderMark from './ProviderMark.vue'
import { call } from '../api'
import { act, busy, providerLabel } from '../store'
import { t } from '../i18n'

const props = defineProps<{ accounts: { provider: string; id: string; name: string; region: string }[] }>()
const emit = defineEmits<{ close: [] }>()
const key = (a: { provider: string; id: string }) => `${a.provider}/${a.id}`
const selected = ref(props.accounts.map(key))
const all = computed(() => props.accounts.length > 0 && selected.value.length === props.accounts.length)
async function save() {
  const accounts = props.accounts.filter(a => selected.value.includes(key(a))).map(({ provider, id }) => ({ provider, id }))
  const result = await act('json-batch-save', () => call('account_json_save_batch', { accounts }), undefined, [])
  if (result) emit('close')
}
</script>

<template>
  <a-modal visible :width="620" :title="t('批量导出账号 JSON', 'Export account JSON in bulk')" :footer="false" @cancel="emit('close')">
    <p class="hint">{{ t('选择当前筛选范围内要导出的账号；文件保留平台和地区，可一次导入多个平台。', 'Choose accounts from the current filters; the file preserves provider and region for batch import.') }}</p>
    <div class="batch-export-head">
      <a-checkbox :model-value="all" :indeterminate="selected.length > 0 && !all" @change="v => selected = v ? accounts.map(key) : []">{{ t('全选', 'Select all') }}</a-checkbox>
      <span class="muted">{{ t(`已选 ${selected.length} / ${accounts.length} 个账号`, `${selected.length} / ${accounts.length} selected`) }}</span>
    </div>
    <a-checkbox-group v-model="selected" class="batch-account-list">
      <a-checkbox v-for="a in accounts" :key="key(a)" :value="key(a)" class="batch-account-option">
        <ProviderMark :provider="a.provider" /><strong>{{ a.name }}</strong><span class="muted">{{ providerLabel(a.provider) }} · {{ a.region === 'intl' ? t('国际版', 'International') : t('国内版', 'China') }}</span>
      </a-checkbox>
    </a-checkbox-group>
    <a-alert type="warning" style="margin-top: 16px">{{ t('导出文件包含完整登录凭据，请妥善保管。', 'The export contains full sign-in credentials. Keep it private.') }}</a-alert>
    <div class="batch-export-footer"><a-button type="primary" :disabled="!selected.length" :loading="busy('json-batch-save')" @click="save"><template #icon><IconDownload /></template>{{ t('导出到「下载」', 'Save to Downloads') }}</a-button></div>
  </a-modal>
</template>

<style scoped>
.batch-export-head { display: flex; justify-content: space-between; align-items: center; margin: 16px 0 8px; }
.batch-account-list { display: flex; flex-direction: column; max-height: 360px; overflow-y: auto; gap: 4px; }
.batch-account-option { padding: 10px 8px; border-radius: 8px; background: var(--color-fill-1); }
.batch-account-option :deep(.arco-checkbox-label) { display: flex; align-items: center; gap: 8px; min-width: 0; flex: 1; }
.batch-account-option strong { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; flex: 1; }
.batch-account-option .muted { font-size: 12px; white-space: nowrap; }
.batch-export-footer { display: flex; justify-content: flex-end; margin-top: 20px; }
</style>
