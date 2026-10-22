<script setup lang="ts">
import ProviderMark from "../components/ProviderMark.vue"
import { computed, ref, watch } from 'vue'
import { Modal, Message } from '@arco-design/web-vue'
import { IconCopy } from '@arco-design/web-vue/es/icon'
import PageHead from '../components/PageHead.vue'
import ApiKeyField from '../components/workbuddy/ApiKeyField.vue'
import RuntimeSettings from '../components/workbuddy/RuntimeSettings.vue'
import { call, type Settings, type ProviderKey } from '../api'
import { t } from '../i18n'
import { state, act, busy, saveSettings, copy, refreshApp, refreshPool, PROVIDERS, providerLabel, regionLabel } from '../store'

const draft = ref<Settings | null>(null)
const dirty = ref(false)
watch(
  () => state.app?.settings,
  (s) => {
    if (s && !dirty.value) draft.value = JSON.parse(JSON.stringify(s))
  },
  { immediate: true, deep: true }
)
const port = computed(() => state.app?.port ?? draft.value?.port ?? 24800)
const base = computed(() => `http://127.0.0.1:${port.value}`)
const strategies = computed(() => [
  { value: 'roundRobin', label: t('轮询', 'Round robin'), desc: t('同一模型按账号顺序逐个使用，每个模型独立推进', 'Rotate accounts per model') },
  { value: 'creditFirst', label: t('积分优先', 'Credits first'), desc: t('剩余积分最多的账号优先；未查询或美元额度排在后面', 'Most remaining credits first; unknown/USD balances last') },
  { value: 'expiryFirst', label: t('临期优先', 'Expiring first'), desc: t('最早到期的积分包所在账号优先，先用掉快过期的积分', 'Use credits that expire soonest first') },
  { value: 'ordered', label: t('顺序优先', 'Ordered'), desc: t('总是使用排在最前面的可用账号', 'Always the first available account') },
  { value: 'random', label: t('随机', 'Random'), desc: t('在符合条件的账号中随机选择', 'Random eligible account') },
])
function save() {
  const d = draft.value!
  return saveSettings(
    { port: d.port, strategy: d.strategy, providers: d.providers, pinned: d.pinned, rejectBrowserOrigin: d.rejectBrowserOrigin, autoStart: d.autoStart },
    t('接入设置已保存，对新请求立即生效', 'Saved; applies to new requests')
  ).then(() => (dirty.value = false))
}
const touch = () => (dirty.value = true)
function toggleRegion(p: ProviderKey, r: 'cn' | 'intl', on: boolean) {
  const s = draft.value!.providers[p]
  s.regions = on ? [...new Set([...s.regions, r])] : s.regions.filter((x) => x !== r)
  touch()
}
const pinScopes = computed(() => [
  { key: 'all', label: t('统一入口 /v1', 'Unified /v1') },
  { key: 'cn', label: t('国内入口 /cn/v1', 'China /cn/v1') },
  { key: 'intl', label: t('国际入口 /intl/v1', 'International /intl/v1') },
  ...PROVIDERS.map((p) => ({ key: p.key, label: `${p.label} /${p.key}/v1` })),
])
const accountOptions = computed(() =>
  Object.entries(state.pool?.runtime ?? {}).map(([key, a]: [string, any]) => ({ value: key, label: `${providerLabel(a.provider)} · ${regionLabel(a.region)} · ${a.name}` }))
)
function setPin(scope: string, value: string) {
  const pinned = { ...draft.value!.pinned }
  if (value) pinned[scope] = value
  else delete pinned[scope]
  draft.value!.pinned = pinned
  touch()
}
// API key
function saveKey(value: string, expected: string) {
  Modal.confirm({
    title: t('确认更换本地 API Key', 'Replace the local API key?'),
    content: t('保存后旧 Key 立即无法发起新请求，请同步更新客户端配置；已开始的请求不受影响。不会修改任何官方账号令牌。', 'The old key stops working for new requests immediately; update your clients. In-flight requests continue. Official tokens are not touched.'),
    onOk: () => act('api-key', () => call('set_api_key', { value, expected }), t('本地 API Key 已更新，请同步修改客户端配置', 'Local API key updated; update your clients'), [refreshApp]),
  })
}
// Client configuration
const client = ref<'claude' | 'codex' | 'openai' | 'zcode'>('claude')
const firstModel = computed(() => state.catalog?.routes.find((r) => r.available)?.alias || '<model alias>')
const quote = (v: string) => "'" + v.replaceAll("'", "'\"'\"'") + "'"
const configCode = computed(() => {
  const m = firstModel.value
  if (client.value === 'claude')
    return `export ANTHROPIC_BASE_URL="${base.value}"\nexport ANTHROPIC_AUTH_TOKEN="<LOCAL_API_KEY>"\nexport ANTHROPIC_MODEL=${quote(m)}\n\nclaude`
  if (client.value === 'codex')
    return `# ~/.codex/config.toml\nmodel = ${JSON.stringify(m)}\nmodel_provider = "agentdock"\n\n[model_providers.agentdock]\nname = "AgentDock"\nbase_url = "${base.value}/v1"\nwire_api = "responses"\nenv_key = "AGENTDOCK_API_KEY"\n\n# ${t('在终端设置，勿写入 TOML', 'Set in your terminal, not in the TOML')}:\n# export AGENTDOCK_API_KEY="<LOCAL_API_KEY>"`
  if (client.value === 'zcode')
    return `Base URL: ${base.value}/zcode\nAPI Key: <LOCAL_API_KEY>\n# Messages → /zcode/v1/messages · Chat → /zcode/chat/completions · Responses → /zcode/responses`
  return `Base URL: ${base.value}/v1\nAPI Key: <LOCAL_API_KEY>\nModel: ${m}\n\ncurl ${base.value}/v1/chat/completions \\\n  -H "Authorization: Bearer $AGENTDOCK_API_KEY" \\\n  -H "Content-Type: application/json" \\\n  -d ${quote(JSON.stringify({ model: m, messages: [{ role: 'user', content: 'Hello' }], stream: true }))}`
})
const endpoints = computed(() => [
  [t('统一入口（全部端、全部地区）', 'Unified (all providers and regions)'), `${base.value}/v1`],
  [t('Claude Code / Anthropic', 'Claude Code / Anthropic'), base.value],
  [t('仅国内账号', 'China accounts only'), `${base.value}/cn/v1`],
  [t('仅国际账号', 'International accounts only'), `${base.value}/intl/v1`],
  ...PROVIDERS.map((p) => [t(`仅 ${p.label}`, `${p.label} only`), `${base.value}/${p.key}/v1`]),
  [t('单端单地区（示例）', 'One provider and region (example)'), `${base.value}/trae/cn/v1`],
  [t('ZCode 专用', 'ZCode profile'), `${base.value}/zcode`],
])
// WorkBuddy runtime settings
const wbSnapshot = computed(() => ({ ...(state.pool?.workbuddy ?? {}), accounts: state.pool?.workbuddy?.accounts ?? [] }))
// Qoder transport
const qoder = ref<any>(null)
watch(
  () => state.pool?.qoder?.gateway,
  (g) => {
    if (g && !qoder.value) qoder.value = { ...g }
  },
  { immediate: true }
)
function saveQoder() {
  return act('qoder-gateway', () => call('qoder_save_gateway', { config: qoder.value }), t('Qoder 传输设置已保存', 'Qoder transport saved'), [refreshPool])
}
</script>

<template>
  <PageHead eyebrow="API CONNECTION" :title="t('接入设置', 'API Access')" :desc="t('一个本地地址接入 WorkBuddy、Qoder 与 Trae 的全部账号；Claude Code、Codex 与 OpenAI 兼容客户端通用。', 'One local address for every WorkBuddy, Qoder and Trae account; works with Claude Code, Codex and OpenAI-compatible clients.')">
    <a-button type="primary" :disabled="!dirty" :loading="busy('settings')" @click="save">{{ t('保存配置', 'Save') }}</a-button>
  </PageHead>
  <div v-if="draft" class="split-2">
    <div>
      <section class="box">
        <div class="box-head"><div><h2>{{ t('网关与调度', 'Gateway and scheduling') }}</h2><p>{{ t('仅监听 127.0.0.1；每个请求调用一个账号。仅当 Qoder 在输出前明确拒绝（401/403/429）或排队时，才换下一个可用账号重试，最多 3 个；已开始输出的请求不会重放。', 'Loopback only; one account per request. Only an explicit Qoder rejection (401/403/429) or queue notice before any output moves the request to the next eligible account, at most 3; started output is never replayed.') }}</p></div></div>
        <div class="field-row">
          <div><b>{{ t('监听端口', 'Port') }}</b><small>{{ t('修改端口需先停止服务', 'Stop the gateway to change it') }}</small></div>
          <div class="control"><a-input-number v-model="draft.port" :min="1024" :max="65535" :disabled="state.app?.running" style="width: 140px" @change="touch" /></div>
        </div>
        <div class="field-row">
          <div><b>{{ t('调度策略', 'Scheduling') }}</b><small>{{ t('同一模型在所有提供它的端、地区和账号之间统一调度', 'One pool per model across every provider, region and account that offers it') }}</small></div>
          <div class="control" style="flex-direction: column; align-items: stretch">
            <a-radio-group v-model="draft.strategy" direction="vertical" @change="touch">
              <a-radio v-for="s in strategies" :key="s.value" :value="s.value"><b>{{ s.label }}</b> <span class="muted" style="font-size: 12px">· {{ s.desc }}</span></a-radio>
            </a-radio-group>
          </div>
        </div>
        <div class="field-row">
          <div><b>{{ t('拒绝网页跨域请求', 'Reject browser origins') }}</b><small>{{ t('带 http(s) Origin 的网页请求返回 403；原生客户端不受影响', 'Requests with an http(s) Origin get 403; native clients are unaffected') }}</small></div>
          <div class="control"><a-switch v-model="draft.rejectBrowserOrigin" @change="touch" /></div>
        </div>
        <div class="field-row">
          <div><b>{{ t('打开应用时自动启动服务', 'Start with the app') }}</b></div>
          <div class="control"><a-switch v-model="draft.autoStart" @change="touch" /></div>
        </div>
      </section>
      <section class="box">
        <div class="box-head"><div><h2>{{ t('参与反代的端与地区', 'Providers and regions') }}</h2><p>{{ t('关闭后该端/地区的账号不再接收新请求；模型目录仍可见。', 'Disabled providers/regions take no new requests; models stay visible.') }}</p></div></div>
        <div v-for="p in PROVIDERS" :key="p.key" class="field-row">
          <div><b><ProviderMark :provider="p.key" style="margin-right: 8px" />{{ p.label }}</b></div>
          <div class="control">
            <a-switch v-model="draft.providers[p.key].enabled" @change="touch" />
            <a-checkbox :model-value="draft.providers[p.key].regions.includes('cn')" :disabled="!draft.providers[p.key].enabled" @change="(v) => toggleRegion(p.key, 'cn', !!v)">{{ t('国内版', 'China') }}</a-checkbox>
            <a-checkbox :model-value="draft.providers[p.key].regions.includes('intl')" :disabled="!draft.providers[p.key].enabled" @change="(v) => toggleRegion(p.key, 'intl', !!v)">{{ t('国际版', 'International') }}</a-checkbox>
            <span class="muted">{{ t('单账号并发', 'Per-account concurrency') }}</span>
            <a-input-number v-model="draft.providers[p.key].concurrency" :min="1" :max="16" size="small" style="width: 90px" @change="touch" />
          </div>
        </div>
      </section>
      <section class="box">
        <div class="box-head"><div><h2>{{ t('固定账号', 'Pinned accounts') }}</h2><p>{{ t('为某个入口固定使用一个账号；模型不在该账号时返回错误，不会越权改用其他账号。请求头 x-account-id 可临时指定。', 'Pin one account to an entry point; requests for models it lacks fail rather than widening the pool. The x-account-id header overrides per request.') }}</p></div></div>
        <div v-for="s in pinScopes" :key="s.key" class="field-row">
          <div><b class="mono" style="font-size: 13px">{{ s.label }}</b></div>
          <div class="control">
            <a-select :model-value="draft.pinned[s.key] || ''" allow-search style="width: 100%" @change="(v) => setPin(s.key, String(v || ''))">
              <a-option value="">{{ t('不固定（按调度策略）', 'Not pinned (use strategy)') }}</a-option>
              <a-option v-for="o in accountOptions" :key="o.value" :value="o.value">{{ o.label }}</a-option>
            </a-select>
          </div>
        </div>
      </section>
      <section class="box">
        <div class="box-head"><div><h2>WorkBuddy</h2><p>{{ t('辅助 Agent、设备 SDK 与积分定时刷新', 'Auxiliary agents, device SDK and scheduled credit refresh') }}</p></div><ProviderMark provider="workbuddy" /></div>
        <RuntimeSettings :snapshot="wbSnapshot as any" :pending="busy('wb-runtime') ? 'runtime-settings' : ''" @save="(settings, confirmed) => act('wb-runtime', () => call('wb_set_runtime_settings', { settings, confirmed }), t('运行设置已保存', 'Runtime settings saved'))" />
      </section>
      <section v-if="qoder" class="box">
        <div class="box-head"><div><h2>Qoder</h2><p>{{ t('推理传输方式与官方地址；修改需先停止服务', 'Inference transport and endpoints; stop the gateway to change them') }}</p></div><ProviderMark provider="qoder" /></div>
        <div class="field-row">
          <div><b>{{ t('传输协议', 'Transport') }}</b><small>{{ t('默认使用本机 Qoder 原生签名；Model Server HTTP 为高级选项', 'Native signing via the installed Qoder by default; Model Server HTTP is advanced') }}</small></div>
          <div class="control">
            <a-select v-model="qoder.transport" style="width: 260px">
              <a-option value="legacy">{{ t('Qoder 原生签名', 'Native signed') }}</a-option>
              <a-option value="modelServer">Model Server HTTP</a-option>
            </a-select>
          </div>
        </div>
        <div class="field-row">
          <div><b>{{ t('国内推理地址', 'China endpoint') }}</b></div>
          <div class="control"><a-input v-model="qoder.cnEndpoint" /></div>
        </div>
        <div class="field-row">
          <div><b>{{ t('国际推理地址', 'International endpoint') }}</b><small>{{ t('仅接受 Qoder 官方 HTTPS 域名', 'Official Qoder HTTPS hosts only') }}</small></div>
          <div class="control"><a-input v-model="qoder.intlEndpoint" /></div>
        </div>
        <div style="text-align: right"><a-button :loading="busy('qoder-gateway')" @click="saveQoder">{{ t('保存 Qoder 设置', 'Save Qoder settings') }}</a-button></div>
      </section>
    </div>
    <div>
      <section class="box">
        <div class="box-head"><div><h2>{{ t('本地 API Key', 'Local API key') }}</h2><p>{{ t('三端共用一个本地密钥，与任何官方令牌无关。', 'One local key for all providers, independent of official tokens.') }}</p></div></div>
        <ApiKeyField :value="state.app?.apiKey ?? ''" :saving="busy('api-key')" @copy="copy" @save="saveKey" />
      </section>
      <section class="box">
        <div class="box-head"><div><h2>{{ t('接入地址', 'Endpoints') }}</h2><p>{{ t('路径前缀可限定端和地区，模型列表随之过滤。', 'Path prefixes restrict providers and regions; /models follows.') }}</p></div></div>
        <div v-for="[label, url] in endpoints" :key="url" class="kv">
          <span>{{ label }}</span>
          <b class="mono" style="display: flex; gap: 6px; align-items: center">{{ url }}<a-button size="mini" type="text" @click="copy(url)"><IconCopy /></a-button></b>
        </div>
      </section>
      <section class="box">
        <div class="box-head">
          <div><h2>{{ t('客户端配置', 'Client configuration') }}</h2></div>
          <a-button size="small" @click="copy(configCode)"><IconCopy /> {{ t('复制', 'Copy') }}</a-button>
        </div>
        <a-radio-group v-model="client" type="button" style="margin-bottom: 12px">
          <a-radio value="claude">Claude Code</a-radio>
          <a-radio value="codex">Codex</a-radio>
          <a-radio value="openai">OpenAI</a-radio>
          <a-radio value="zcode">ZCode</a-radio>
        </a-radio-group>
        <div class="code-block">{{ configCode }}</div>
        <p class="hint">
          {{
            client === 'codex'
              ? t('Responses 为无状态兼容：回传完整历史；支持 function、namespace 与 custom 工具；不支持服务端工具、previous_response_id 或跨提供商加密推理。', 'Stateless Responses: send full history. Function, namespace and custom tools are mapped; no hosted tools, previous_response_id or cross-provider encrypted reasoning.')
              : client === 'claude'
                ? t('用本地 Key 替换 <LOCAL_API_KEY>，模型填写模型目录中的别名。count_tokens 为本地估算。', 'Replace <LOCAL_API_KEY> and use a catalog alias. count_tokens is a local estimate.')
                : client === 'zcode'
                  ? t('ZCode 可选择 Messages、Chat Completions 或 Responses，三种格式均使用此地址；记录标记为 ZCode。', 'ZCode may use Messages, Chat Completions or Responses at this address; requests are labelled ZCode.')
                  : t('在任意兼容 OpenAI 的客户端填入地址和本地密钥。', 'Use the base URL and local key in any OpenAI-compatible client.')
          }}
        </p>
        <p class="hint">{{ t('Trae 原生接口目前只支持纯文本对话；带工具或图片的请求会在调度到 Trae 账号时返回明确错误，不会伪造结果。', 'Trae native chat supports plain text only; tool or image requests routed to Trae fail explicitly.') }}</p>
      </section>
    </div>
  </div>
</template>
