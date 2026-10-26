<script setup lang="ts">
import { isWindows } from '../../api'
import { t, fmtDateTime } from '../../i18n'
import { ref, watch } from 'vue'
import { InputNumber as AInputNumber, Modal, Message } from '@arco-design/web-vue'
import type { RuntimeSettings, Snapshot } from '../../wb-api'
const props = defineProps<{ snapshot: Snapshot; pending: string }>()
const emit = defineEmits<{ save: [settings: RuntimeSettings, confirmed: boolean] }>()
const draft = ref<RuntimeSettings>({
  accountRefreshEnabled: false,
  accountRefreshMinutes: 30,
  allowAuxiliaryAgents: true,
  useDeviceSdk: false,
  tokenAutoRenew: true,
})
watch(
  () => JSON.stringify(props.snapshot.settings),
  (v) => {
    if (v) draft.value = { tokenAutoRenew: true, useDeviceSdk: false, ...JSON.parse(v) }
  },
  { immediate: true }
)
const time = (v?: number) => (v ? fmtDateTime(v) : t('尚未执行', 'Not yet'))
function save() {
  if (
    !Number.isInteger(draft.value.accountRefreshMinutes) ||
    draft.value.accountRefreshMinutes < 5 ||
    draft.value.accountRefreshMinutes > 1440
  ) {
    Message.warning(t('请填写 5 至 1440 分钟的整数间隔', 'Enter an interval of 5–1440 minutes'))
    return
  }
  const settings = { ...draft.value }
  const enablingSdk = settings.useDeviceSdk && !props.snapshot.settings?.useDeviceSdk
  if (!settings.accountRefreshEnabled && !enablingSdk) return emit('save', settings, false)
  Modal.confirm({
    title: t('确认运行设置', 'Confirm runtime settings'),
    content: [
      settings.useDeviceSdk
        ? isWindows
          ? t('设备 SDK 开启：启动反代时预热设备令牌；不保存系统密码，失败不自动降级。', 'Device SDK on: the device token is prepared when the gateway starts; no system password is stored and failures never downgrade silently.')
          : t('设备 SDK 开启：启动反代时预热设备令牌，可能触发 macOS 钥匙串密码授权；不保存系统密码，失败不自动降级。', 'Device SDK on: the device token is prepared when the gateway starts and may trigger a macOS Keychain prompt; no system password is stored and failures never downgrade silently.')
        : t('设备 SDK 关闭：不获取设备令牌，上游可能拒绝。', 'Device SDK off: no device token is sent; the upstream may reject requests.'),
      settings.accountRefreshEnabled
        ? t(`定时刷新开启：每 ${settings.accountRefreshMinutes} 分钟向官方查询所有启用账号的积分与积分包，首次在一个完整间隔后执行；Token 按自动续期设置独立处理，可能遇到限流。`, `Scheduled refresh on: every ${settings.accountRefreshMinutes} minutes, credits and packs of enabled accounts are queried (first run after one full interval), with token renewal controlled independently; rate limits may apply.`)
        : t('定时刷新保持关闭。', 'Scheduled refresh stays off.'),
    ].join(' '),
    okText: t('确认并保存', 'Confirm and save'),
    onOk: () => emit('save', settings, true),
  })
}
</script>
<template>
  <section class="panel runtime-settings">
    <div class="panel-heading">
      <div>
        <h2>{{ t('账号维护与续期', 'Account maintenance and renewal') }}</h2>
        <p>{{ t('Token 自动续期与积分定时刷新分别设置。', 'Configure token renewal and credit refresh independently.') }}</p>
      </div>
      <a-tag :color="snapshot.settings?.accountRefreshEnabled ? 'green' : 'gray'">{{
        snapshot.settings?.accountRefreshEnabled ? t('积分刷新已开启', 'Credit refresh on') : t('积分刷新已关闭', 'Credit refresh off')
      }}</a-tag>
    </div>
    <div class="runtime-row">
      <div>
        <strong>{{ t('Token 自动续期', 'Automatic token renewal') }}</strong>
        <p>{{ t('默认开启。应用运行时每 5 分钟检查，临近到期或距上次续期满 24 小时时续期；请求遇到 401 时尝试续期一次。已失效的刷新凭据仍需重新登录。', 'On by default. Checks every 5 minutes while running, renews near expiry or after 24 hours, and tries once after HTTP 401. Invalid refresh credentials require login.') }}</p>
      </div>
      <a-switch v-model="draft.tokenAutoRenew" :aria-label="t('Token 自动续期', 'Automatic token renewal')" />
    </div>
    <div class="runtime-row">
      <div>
        <strong>{{ t('启用定时刷新', 'Enable scheduled refresh') }}</strong>
        <p>{{ t('仅在应用运行时执行，范围为所有开启反代的账号。', 'Runs only while the app is open, for every proxy-enabled account.') }}</p>
      </div>
      <a-switch v-model="draft.accountRefreshEnabled" />
    </div>
    <div class="runtime-row">
      <div>
        <strong>{{ t('刷新间隔', 'Interval') }}</strong>
        <p>{{ t('重启应用或修改设置后，从下一个完整间隔开始计时。', 'After a restart or change, timing starts from the next full interval.') }}</p>
      </div>
      <div class="interval-input">
        <a-input-number
          v-model="draft.accountRefreshMinutes"
          :min="5"
          :max="1440"
          :precision="0"
          :disabled="!draft.accountRefreshEnabled"
        /><span>{{ t('分钟', 'min') }}</span>
      </div>
    </div>
    <div class="schedule-status">
      <span
        >{{ t('下次刷新', 'Next') }}
        <b>{{
          snapshot.settings?.accountRefreshEnabled
            ? time(snapshot.accountRefresh?.nextAt)
            : t('未安排', 'Not scheduled')
        }}</b></span
      ><span
        >{{ t('上次刷新', 'Last') }} <b>{{ time(snapshot.accountRefresh?.lastAt) }}</b></span
      ><span>{{
        snapshot.accountRefresh?.running
          ? t('正在刷新…', 'Refreshing…')
          : t(`成功 ${snapshot.accountRefresh?.success ?? 0} · 失败 ${snapshot.accountRefresh?.failed ?? 0}`, `${snapshot.accountRefresh?.success ?? 0} ok · ${snapshot.accountRefresh?.failed ?? 0} failed`)
      }}</span>
    </div>
    <div class="runtime-row agent-row">
      <div>
        <strong>{{ t('允许客户端辅助 Agent 请求', 'Allow auxiliary agent requests') }}</strong>
        <p>
          {{ t('放行带官方用途标记的标题、摘要、记忆选择等独立请求。关闭后本地返回明确错误，普通对话继续使用。', 'Allow separate requests tagged with official purposes (titles, summaries, memory selection…). When off they fail locally; normal conversations continue.') }}
        </p>
        <small>{{ t('当前反代直接发送聊天请求，不运行 WorkBuddy 的前置 Agent；此开关不会关闭官方服务端审查。未标注用途的客户端后台请求无法据此识别。', 'The gateway sends chat requests directly without WorkBuddy pre-agents; this switch does not disable official server-side review. Untagged background requests cannot be identified.') }}</small>
      </div>
      <a-switch v-model="draft.allowAuxiliaryAgents" />
    </div>
    <div class="runtime-row">
      <div>
        <strong>{{ t('使用设备 SDK（默认关闭）', 'Use device SDK (off by default)') }}</strong>
        <p>
          {{ t('关闭时不加载原生设备 SDK、不获取或发送设备令牌，仍使用本机 Node 进行 gzip 压缩与传输。', 'When off, the native device SDK is not loaded and no device token is sent; the local Node runtime still compresses and transports requests.') }}
        </p>
        <small>{{ isWindows ? '' : t('开启可能触发 macOS 钥匙串授权。', 'Turning it on may prompt for Keychain access. ') }}{{ t('关闭后上游可能拒绝请求；不会伪造令牌或自动重新开启。保存后对新请求生效，进行中的请求不受影响。', 'When off the upstream may reject requests; tokens are never forged or re-enabled automatically. Applies to new requests.') }}</small>
      </div>
      <a-switch v-model="draft.useDeviceSdk" />
    </div>
    <div class="runtime-save">
      <span>{{ t('定时刷新与每日签到独立设置，不改写原客户端凭据。', 'Scheduled refresh and daily check-in are independent and never rewrite client credentials.') }}</span
      ><a-button type="primary" :loading="pending === 'runtime-settings'" @click="save"
        >{{ t('保存运行设置', 'Save runtime settings') }}</a-button
      >
    </div>
  </section>
  <section class="panel runtime-settings profile-panel">
    <div class="panel-heading">
      <div>
        <h2>{{ t('官方请求配置', 'Official request profile') }}</h2>
        <p>
          {{ t('通过后台 helper 使用本机 WorkBuddy 5.9.2 的 Node；设备 SDK 按运行设置选用，不启动官方应用窗口。', 'A background helper uses the installed WorkBuddy 5.9.2 Node runtime; the device SDK follows the runtime setting; no official window is opened.') }}
        </p>
      </div>
      <a-tag color="green">gzip</a-tag>
    </div>
    <div class="profile-grid">
      <span
        >WorkBuddy <b>{{ snapshot.officialProfile?.desktopVersion || '5.9.2' }}</b></span
      >
      <span
        >CLI <b>{{ snapshot.officialProfile?.cliVersion || '2.165.0' }}</b></span
      >
      <span>IDE <b>WorkBuddy / 5.9.2</b></span>
      <span>{{ t('会话追踪', 'Session tracing') }} <b>{{ t('会话与轮次关联 · W3C / B3', 'Session and turn linking · W3C / B3') }}</b></span>
      <span
        >{{ t('设备令牌', 'Device token') }}
        <b>{{
          snapshot.settings?.useDeviceSdk ? t('SDK 已开启 · 内存缓存 5 分钟', 'SDK on · cached 5 min in memory') : t('已关闭 · 不发送', 'Off · not sent')
        }}</b></span
      >
      <span>{{ t('压缩', 'Compression') }} <b>Node JSON / zlib gzip</b></span>
    </div>
    <code>{{
      snapshot.officialProfile?.userAgent || 'WorkBuddy/5.9.2 WorkBuddy/5.9.2 CLI/2.165.0'
    }}</code
    ><small>{{ t('需要安装对应版本及区域的 WorkBuddy；设备令牌获取失败时会返回明确错误。客户端的消息、工具和模型选择按请求保留，桌面端独有的 Agent 指令与行为事件随实际来源提供。', 'Requires the matching WorkBuddy version and region; a failed device token returns a clear error. Client messages, tools and model choice are kept per request.') }}</small>
  </section>
</template>
<style scoped>
.runtime-settings {
  padding: 28px;
  margin-bottom: 24px;
}
.runtime-row {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 28px;
  padding: 22px 0;
  border-top: 1px solid var(--color-neutral-3);
}
.runtime-row p {
  margin: 7px 0 0;
  color: var(--c-84928d);
  line-height: 1.7;
}
.runtime-row small,
.profile-panel small {
  display: block;
  color: var(--c-8b9893);
  line-height: 1.7;
  margin-top: 8px;
}
.runtime-row > div {
  max-width: 820px;
}
.interval-input {
  display: flex;
  align-items: center;
  gap: 10px;
  min-width: 200px;
}
.interval-input .arco-input-number {
  width: 140px;
}
.interval-input span {
  color: var(--c-8b9893);
}
.schedule-status {
  display: flex;
  flex-wrap: wrap;
  gap: 20px;
  background: var(--c-f5f8f6);
  border-radius: 10px;
  padding: 15px;
  color: var(--c-7c8d84);
  font-size: 12px;
}
.schedule-status b {
  font-weight: 500;
  color: var(--c-41574b);
  margin-left: 8px;
}
.agent-row {
  margin-top: 20px;
}
.runtime-save {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 20px;
  color: var(--c-8b9893);
  font-size: 12px;
}
.profile-grid {
  display: flex;
  gap: 28px;
  flex-wrap: wrap;
  font-size: 13px;
  color: var(--c-8b9893);
}
.profile-grid b {
  color: var(--c-456355);
  margin-left: 10px;
}
.profile-panel code {
  display: block;
  overflow-wrap: anywhere;
  padding: 17px;
  background: var(--c-f5f8f6);
  border-radius: 10px;
  margin-top: 20px;
  font-size: 12px;
}
</style>
