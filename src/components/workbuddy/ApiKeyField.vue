<script setup lang="ts">
import { t } from '../../i18n'
import { ref, watch, computed } from 'vue'
import { Message } from '@arco-design/web-vue'
import { IconCopy, IconRefresh, IconEdit } from '@arco-design/web-vue/es/icon'
import { call, desktop } from '../../wb-api'
import { apiKeyIssue } from '../../lib/api-key'
const props = withDefaults(
  defineProps<{
    value: string
    saving: boolean
    available?: boolean
    generateKey?: () => Promise<string>
  }>(),
  { available: desktop }
)
const emit = defineEmits<{ save: [value: string, expected: string]; copy: [value: string] }>()
const reveal = ref(false),
  editing = ref(false),
  generating = ref(false)
const draft = ref(''),
  expected = ref(''),
  showDraft = ref(false)
const issue = computed(() => apiKeyIssue(draft.value))
function edit() {
  expected.value = props.value
  draft.value = props.value
  showDraft.value = false
  editing.value = true
}
function cancel() {
  editing.value = false
  draft.value = ''
  expected.value = ''
  showDraft.value = false
}
async function generate() {
  if (generating.value || props.saving) return
  if (!editing.value) edit()
  generating.value = true
  try {
    draft.value = await (props.generateKey ? props.generateKey() : call<string>('generate_api_key'))
  } catch {
    Message.error(t('随机密钥生成失败，请稍后重试', 'Could not generate a key; try again'))
  } finally {
    generating.value = false
  }
}
watch(
  () => props.value,
  (value) => {
    if (editing.value && value !== expected.value && value === draft.value) {
      cancel()
      reveal.value = false
    }
  }
)
</script>
<template>
  <div class="setting-field api-key-field">
    <label>API Key</label>
    <div class="key-current">
      <code :class="{ masked: !reveal }">{{ reveal ? value || t('请在桌面版获取', 'Open the desktop app') : (value.slice(0, 3) || 'ad-') + '••••••••••••••••••••••••••••••••' }}</code>
      <a-button type="text" @click="reveal = !reveal">{{ reveal ? t('隐藏', 'Hide') : t('显示', 'Show') }}</a-button>
      <a-button :disabled="!value" :aria-label="t('复制 API Key', 'Copy API key')" @click="emit('copy', value)"
        ><IconCopy
      /></a-button>
      <a-button :disabled="!available || saving || generating" @click="edit"
        ><template #icon><IconEdit /></template>{{ t('编辑', 'Edit') }}</a-button
      >
      <a-button :disabled="!available || saving" :loading="generating" @click="generate"
        ><template #icon><IconRefresh /></template>{{ t('随机生成', 'Generate') }}</a-button
      >
    </div>
    <small>{{ t('仅用于本机网关鉴权，不是任何官方令牌。随机生成只更新草稿，保存后才生效。', 'Local gateway authentication only, never an official token. Generating only fills the draft; it applies after saving.') }}</small>
    <section v-if="editing" class="key-editor">
      <label for="local-api-key-draft">{{ t('新的 API Key', 'New API key') }}</label>
      <div class="key-input-row">
        <a-input
          id="local-api-key-draft"
          v-model="draft"
          :type="showDraft ? 'text' : 'password'"
          :disabled="saving || generating"
          :max-length="256"
          autocomplete="new-password"
          spellcheck="false"
          :placeholder="t('输入新的本地 API Key（16–256 字符）', 'New local API key (16–256 characters)')"
          :aria-label="t('新的本地 API Key', 'New local API key')"
        />
        <a-button :disabled="saving" @click="showDraft = !showDraft">{{
          showDraft ? t('隐藏', 'Hide') : t('显示', 'Show')
        }}</a-button>
      </div>
      <p v-if="issue" class="key-error" role="alert">{{ issue }}</p>
      <p v-if="value !== expected" class="key-error" role="alert">
        {{ t('当前密钥已变化，请取消后重新编辑。', 'The key changed meanwhile; cancel and edit again.') }}
      </p>
      <p class="key-warning">
        {{ t('保存将立即使旧 Key 无法发起新请求，请同步更新 Codex 等客户端配置。已开始的请求不会因此中断。', 'Saving stops the old key for new requests immediately; update Codex and other clients. In-flight requests continue.') }}
      </p>
      <div class="key-buttons">
        <a-button :disabled="saving || generating" @click="cancel">{{ t('取消', 'Cancel') }}</a-button>
        <a-button
          type="primary"
          :loading="saving"
          :disabled="!!issue || generating || draft === value || value !== expected"
          @click="emit('save', draft, expected)"
          >{{ t('确认保存', 'Save') }}</a-button
        >
      </div>
    </section>
  </div>
</template>
<style scoped>
.key-current {
  display: flex;
  flex-wrap: wrap;
  gap: 8px;
  align-items: center;
}
.key-current code {
  overflow-wrap: anywhere;
  min-width: 0;
}
.key-current code.masked {
  flex: 1 1 120px;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}
.key-editor {
  margin-top: 14px;
  padding: 16px;
  background: var(--c-f5faf7);
  border: 1px solid var(--c-dcebe2);
  border-radius: 10px;
}
.key-editor > label {
  display: block;
  margin-bottom: 10px;
  font-size: 12px;
}
.key-input-row {
  display: flex;
  align-items: center;
  gap: 8px;
}
.key-input-row .arco-input-wrapper {
  flex: 1;
  min-width: 0;
}
.key-editor p {
  font-size: 12px;
  line-height: 1.7;
  margin: 10px 0;
}
.key-error {
  color: var(--c-b24e42);
}
.key-warning {
  color: var(--c-8b6b36);
}
.key-buttons {
  display: flex;
  justify-content: flex-end;
  gap: 8px;
  margin-top: 12px;
}
</style>
