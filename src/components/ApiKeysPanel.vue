<script setup lang="ts">
import ProviderMark from './ProviderMark.vue'
import { computed, ref } from 'vue'
import { Message, Modal } from '@arco-design/web-vue'
import { IconCopy, IconDelete, IconEdit, IconPlus, IconRefresh } from '@arco-design/web-vue/es/icon'
import { call, type ApiKeyInfo, type ProviderKey, type Region, type Strategy } from '../api'
import { t } from '../i18n'
import { state, act, busy, copy, refreshApp, keyHint, PROVIDERS, regionLabel } from '../store'
import { apiKeyIssue } from '../lib/api-key'

const props = defineProps<{ strategies: Array<{ value: string; label: string }> }>()
const keys = computed(() => state.app?.apiKeys ?? [])
const shown = ref(new Set<string>())
const global = computed(() => state.app?.settings)

const strategyLabel = (s: Strategy | null) =>
  s ? (props.strategies.find((x) => x.value === s)?.label ?? s) : t('跟随全局', 'Global default')
function scopeLabel(k: ApiKeyInfo) {
  if (!k.scope) return t('跟随全局', 'Global default')
  const parts = PROVIDERS.flatMap((p) =>
    (k.scope?.[p.key] ?? []).map((r) => `${p.label} ${regionLabel(r)}`)
  )
  return parts.join(' · ') || t('无', 'None')
}
const globallyOn = (p: ProviderKey, r: Region) =>
  !!global.value?.providers[p]?.enabled && !!global.value?.providers[p]?.regions.includes(r)

type Draft = {
  id: string | null
  alias: string
  enabled: boolean
  /** '' follows the global strategy. */
  strategy: Strategy | ''
  scoped: boolean
  scope: Record<ProviderKey, Region[]>
  /** Current value (existing keys) — guards against stale edits. */
  expected: string
  value: string
  showValue: boolean
}
const editing = ref<Draft | null>(null)
function open(k: ApiKeyInfo | null) {
  const scope = Object.fromEntries(
    PROVIDERS.map((p) => [
      p.key,
      k?.scope
        ? [...(k.scope[p.key] ?? [])]
        : (['cn', 'intl'] as Region[]).filter((r) => globallyOn(p.key, r)),
    ])
  ) as Record<ProviderKey, Region[]>
  editing.value = {
    id: k?.id ?? null,
    alias: k?.alias ?? '',
    enabled: k?.enabled ?? true,
    strategy: k?.strategy ?? '',
    scoped: !!k?.scope,
    scope,
    expected: k?.key ?? '',
    value: k?.key ?? '',
    showValue: false,
  }
}
function toggle(r: Region, p: ProviderKey, on: boolean) {
  const s = editing.value!.scope
  s[p] = on ? [...new Set([...s[p], r])] : s[p].filter((x) => x !== r)
}
const valueIssue = computed(() => {
  const e = editing.value
  if (!e || (!e.id && !e.value.trim())) return ''
  return apiKeyIssue(e.value.trim())
})
async function generate() {
  try {
    editing.value!.value = await call<string>('generate_api_key')
    editing.value!.showValue = true
  } catch {
    Message.error(t('随机密钥生成失败，请稍后重试', 'Could not generate a key; try again'))
  }
}
async function save() {
  const e = editing.value!
  if (valueIssue.value) return Message.error(valueIssue.value)
  const scope = e.scoped
    ? Object.fromEntries(Object.entries(e.scope).filter(([, r]) => r.length))
    : null
  if (scope && !Object.keys(scope).length)
    return Message.error(t('请至少选择一个端与地区', 'Select at least one provider and region'))
  const fields = { alias: e.alias.trim(), enabled: e.enabled, strategy: e.strategy || null, scope }
  const rotate = !!e.id && e.value.trim() !== e.expected
  const run = () =>
    act(
      'api-keys',
      async () => {
        if (!e.id) return call('create_api_key', { value: e.value.trim() || null, fields })
        await call('update_api_key', { id: e.id, fields })
        if (rotate) await call('set_api_key', { id: e.id, value: e.value.trim(), expected: e.expected })
        return { ok: true }
      },
      rotate
        ? t('API Key 已更新，请同步修改客户端配置', 'API key updated; update your clients')
        : t('API Key 已保存，对新请求立即生效', 'API key saved; applies to new requests'),
      [refreshApp]
    ).then((ok) => {
      if (ok !== undefined) editing.value = null
    })
  if (!rotate) return run()
  Modal.confirm({
    title: t('确认更换 Key 值', 'Replace the key value?'),
    content: t(
      '保存后旧值立即无法发起新请求，请同步更新使用它的客户端；已开始的请求不受影响。',
      'The old value stops working for new requests immediately; update clients that use it. In-flight requests continue.'
    ),
    onOk: run,
  })
}
function remove(k: ApiKeyInfo) {
  Modal.confirm({
    title: t('删除 API Key', 'Delete API key'),
    content: t(
      `删除「${k.alias || keyHint(k.key)}」后，使用它的客户端立即无法发起新请求。历史请求记录保留。`,
      `Clients using "${k.alias || keyHint(k.key)}" stop working immediately. Request history is kept.`
    ),
    onOk: () =>
      act('api-keys', () => call('delete_api_key', { id: k.id }), t('已删除', 'Deleted'), [refreshApp]),
  })
}
function setEnabled(k: ApiKeyInfo, enabled: boolean) {
  return act(
    'api-keys',
    () =>
      call('update_api_key', {
        id: k.id,
        fields: { alias: k.alias, enabled, strategy: k.strategy, scope: k.scope },
      }),
    enabled ? t('已启用', 'Enabled') : t('已停用', 'Disabled'),
    [refreshApp]
  )
}
function reveal(id: string) {
  const next = new Set(shown.value)
  next.has(id) ? next.delete(id) : next.add(id)
  shown.value = next
}
</script>

<template>
  <div class="key-list">
    <div v-for="(k, i) in keys" :key="k.id" class="key-item" :class="{ off: !k.enabled }">
      <div class="key-main">
        <div class="key-title">
          <b>{{ k.alias || keyHint(k.key) }}</b>
          <span v-if="i === 0" class="rtag" :title="t('同步写入数据目录的 api-key 文件', 'Mirrored to the api-key file')">{{
            t('主 Key', 'Primary')
          }}</span>
          <span v-if="!k.enabled" class="state-pill bad">{{ t('已停用', 'Disabled') }}</span>
        </div>
        <code class="key-value">{{ shown.has(k.id) ? k.key : k.key.slice(0, 3) + '••••••••••••••••••••' }}</code>
        <div class="key-meta">
          <span>{{ t('调度：', 'Strategy: ') }}{{ strategyLabel(k.strategy) }}</span>
          <span>{{ t('范围：', 'Scope: ') }}{{ scopeLabel(k) }}</span>
        </div>
      </div>
      <div class="key-actions">
        <a-switch size="small" :model-value="k.enabled" :loading="busy('api-keys')" @change="(v) => setEnabled(k, !!v)" />
        <a-button size="mini" type="text" @click="reveal(k.id)">{{
          shown.has(k.id) ? t('隐藏', 'Hide') : t('显示', 'Show')
        }}</a-button>
        <a-button size="mini" :aria-label="t('复制 API Key', 'Copy API key')" @click="copy(k.key)"><IconCopy /></a-button>
        <a-button size="mini" @click="open(k)"><template #icon><IconEdit /></template>{{ t('编辑', 'Edit') }}</a-button>
        <a-button size="mini" status="danger" :disabled="keys.length <= 1" @click="remove(k)"><IconDelete /></a-button>
      </div>
    </div>
    <a-button long @click="open(null)"><template #icon><IconPlus /></template>{{ t('新建 API Key', 'New API key') }}</a-button>
    <small class="muted">{{
      t(
        '仅用于本机网关鉴权，不是任何官方令牌。每个 Key 可单独设置调度策略与可调用的端和地区（只能在全局范围内收窄）；请求记录与运行日志会标明所用 Key。',
        'Local gateway authentication only, never an official token. Each key may set its own strategy and providers/regions (narrowing the global scope only); requests and runtime logs show the key used.'
      )
    }}</small>
  </div>

  <a-modal
    v-if="editing"
    :visible="true"
    :width="620"
    :title="editing.id ? t('编辑 API Key', 'Edit API key') : t('新建 API Key', 'New API key')"
    :ok-loading="busy('api-keys')"
    @ok="save"
    @cancel="editing = null"
  >
    <div class="key-form">
      <label>{{ t('别名', 'Alias') }}</label>
      <a-input v-model="editing.alias" :max-length="64" :placeholder="t('如：Codex、团队 A；留空显示 Key 缩写', 'e.g. Codex, team A; empty shows the key hint')" />

      <label>{{ t('Key 值', 'Key value') }}</label>
      <div class="key-input-row">
        <a-input
          v-model="editing.value"
          :type="editing.showValue ? 'text' : 'password'"
          :max-length="256"
          autocomplete="new-password"
          spellcheck="false"
          :placeholder="editing.id ? '' : t('留空自动随机生成', 'Leave empty to generate')"
        />
        <a-button @click="editing.showValue = !editing.showValue">{{ editing.showValue ? t('隐藏', 'Hide') : t('显示', 'Show') }}</a-button>
        <a-button @click="generate"><template #icon><IconRefresh /></template>{{ t('随机生成', 'Generate') }}</a-button>
      </div>
      <p v-if="valueIssue" class="key-error">{{ valueIssue }}</p>
      <p v-else-if="editing.id && editing.value.trim() !== editing.expected" class="key-warning">
        {{ t('保存后旧值立即失效，请同步更新客户端。', 'The old value stops working once saved; update your clients.') }}
      </p>

      <label>{{ t('调度策略', 'Strategy') }}</label>
      <a-select v-model="editing.strategy">
        <a-option value="">{{ t('跟随全局（', 'Global default (') }}{{ strategyLabel(global?.strategy ?? null) }}）</a-option>
        <a-option v-for="s in strategies" :key="s.value" :value="s.value">{{ s.label }}</a-option>
      </a-select>

      <label>{{ t('可调用的端与地区', 'Providers and regions') }}</label>
      <a-radio-group v-model="editing.scoped" type="button">
        <a-radio :value="false">{{ t('跟随全局', 'Global default') }}</a-radio>
        <a-radio :value="true">{{ t('自定义', 'Custom') }}</a-radio>
      </a-radio-group>
      <div v-if="editing.scoped" class="key-scope">
        <div v-for="p in PROVIDERS" :key="p.key" class="key-scope-row">
          <b><ProviderMark :provider="p.key" style="margin-right: 6px" />{{ p.label }}</b>
          <a-checkbox
            v-for="r in ['cn', 'intl'] as Region[]"
            :key="r"
            :model-value="editing.scope[p.key].includes(r)"
            :disabled="!globallyOn(p.key, r) && !editing.scope[p.key].includes(r)"
            @change="(v) => toggle(r, p.key, !!v)"
            >{{ regionLabel(r)
            }}<small v-if="!globallyOn(p.key, r)" class="muted">{{ t('（全局未开启）', ' (off globally)') }}</small></a-checkbox
          >
        </div>
        <p class="hint">{{ t('全局「参与反代的端与地区」中关闭的端或地区，对所有 Key 都不生效。', 'Providers/regions disabled globally stay off for every key.') }}</p>
      </div>

      <a-checkbox v-model="editing.enabled">{{ t('启用此 Key', 'Enable this key') }}</a-checkbox>
    </div>
  </a-modal>
</template>

<style scoped>
.key-list {
  display: flex;
  flex-direction: column;
  gap: 10px;
}
.key-item {
  display: flex;
  justify-content: space-between;
  gap: 12px;
  padding: 12px 14px;
  border: 1px solid var(--line);
  border-radius: 12px;
}
.key-item.off {
  opacity: 0.6;
}
.key-main {
  min-width: 0;
  display: flex;
  flex-direction: column;
  gap: 4px;
}
.key-title {
  display: flex;
  gap: 8px;
  align-items: center;
}
.key-value {
  font-size: 12px;
  color: var(--text-2);
  overflow-wrap: anywhere;
}
.key-meta {
  display: flex;
  flex-wrap: wrap;
  gap: 4px 14px;
  font-size: 12px;
  color: var(--muted);
}
.key-actions {
  display: flex;
  gap: 6px;
  align-items: flex-start;
  flex: none;
}
.key-form {
  display: flex;
  flex-direction: column;
  gap: 8px;
}
.key-form > label {
  margin-top: 6px;
  font-size: 12px;
  color: var(--muted);
}
.key-input-row {
  display: flex;
  gap: 8px;
}
.key-input-row .arco-input-wrapper {
  flex: 1;
  min-width: 0;
}
.key-scope {
  display: flex;
  flex-direction: column;
  gap: 6px;
  padding: 10px 12px;
  border: 1px solid var(--line);
  border-radius: 10px;
}
.key-scope-row {
  display: flex;
  align-items: center;
  gap: 14px;
}
.key-scope-row b {
  width: 120px;
}
.key-error,
.key-warning {
  font-size: 12px;
  margin: 0;
}
.key-error {
  color: var(--c-b24e42);
}
.key-warning {
  color: var(--c-8b6b36);
}
</style>
