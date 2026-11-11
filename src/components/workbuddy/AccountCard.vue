<script setup lang="ts">
import { revealPhones, setAccountDisabled } from '../../store'
import { computed } from 'vue'
import ProviderMark from '../ProviderMark.vue'
import { IconMobile,
  IconPlayCircle,
  IconStop,
  IconCalendar,
  IconClockCircle,
  IconThunderbolt,
  IconMore,
  IconSend,
  IconStar,
  IconCheck,
  IconCode,
  IconRefresh,
  IconInfoCircle,
  IconArrowRight,
} from '@arco-design/web-vue/es/icon'
import type { Account, Snapshot } from '../../wb-api'
import { hasFullPhone, phoneText,
  accountName,
  credits,
  resourceRows,
  resourceName,
  expiry,
  remaining,
  updated,
} from '../../lib/account-display'
import { t } from '../../i18n'
import AccountAuthAlert from '../AccountAuthAlert.vue'
import AccountDisabledBar from '../AccountDisabledBar.vue'
const props = defineProps<{
  account: Account
  clients: Snapshot['clients']
  now: number
  busy: boolean
  creditLoading: boolean
  selected?: boolean
  compact: boolean
}>()
const emit = defineEmits<{
  enable: [boolean]
  checkinEnable: [boolean]
  checkin: []
  refresh: []
  renew: []
  info: []
  resources: []
  switch: [string]
  json: []
  relogin: []
  remove: []
  select: [boolean]
}>()
const name = computed(() => accountName(props.account, revealPhones.value))
const expired = computed(() => !!props.account.expiresAt && props.account.expiresAt <= props.now)
const loginInvalid = computed(() => expired.value || props.account.authFailed || props.account.credit?.authFailed || props.account.presentation?.needsRelogin)
const rows = computed(() => resourceRows(props.account.credit?.resources, props.now))
const limits = computed(() =>
  (props.account.modelLimits ?? [])
    .filter((l) => l.resetAt > props.now)
    .slice()
    .sort((a, b) => a.resetAt - b.resetAt)
)
const active = computed(() => props.account.presentation?.clients ?? {})
const uid = computed(
  () => props.account.presentation?.uid || props.account.id.split(':').slice(1).join(':')
)
const near = (t: number | null) => t !== null && t - props.now < 7 * 86400000
const travel = computed(() => props.account.presentation?.travel)
const travelTip = computed(() =>
  travel.value?.label === 'traveling'
    ? `${t('旅行中', 'Traveling')} · ${travel.value.locationName ?? ''} · ${travel.value.arriveAt ? expiry(travel.value.arriveAt < 1e12 ? travel.value.arriveAt * 1000 : travel.value.arriveAt) : ''}`
    : travel.value?.label === 'finished'
      ? t(`旅行已结束 · ${travel.value.rewardCredit ?? 0} 积分`, `Trip finished · ${travel.value.rewardCredit ?? 0} credits`)
      : t('尚未旅行', 'Not traveled')
)
const color = computed(() => Array.from(name.value).reduce((n, c) => n + c.charCodeAt(0), 0) % 3)
const targets = computed(() => [
  { id: 'workbuddy', name: 'WorkBuddy', mark: 'W', available: true },
  {
    id: 'ide',
    name: 'IDE',
    mark: 'I',
    available: (props.account.variant === 'ai' ? props.clients?.aiIde : props.clients?.cnIde)
      ?.available,
  },
  { id: 'vscode', name: 'VS Code', mark: '‹›', available: props.clients?.vscode?.available },
  { id: 'cli', name: 'CLI', mark: '>_', available: props.clients?.cli?.available },
])
</script>
<template>
  <article
    class="switch-card wb-account-card"
    :class="[
      { 'current-card': active.workbuddy, 'compact-card': compact, 'auth-invalid-card': loginInvalid && !account.disabled, 'account-disabled-card': account.disabled },
      `tone-${color}`,
    ]"
  >
    <header class="switch-card-header">
      <div class="switch-avatar">{{ name.slice(0, 1) }}</div>
      <div class="switch-identity">
        <h3 :title="name">{{ name }}</h3>
        <p :title="uid">UID · {{ uid }}</p>
        <p v-if="account.presentation?.phoneNumber" class="switch-contact mono"><IconMobile /> {{ phoneText(account.presentation.phoneNumber, revealPhones) }} <span v-if="revealPhones && !hasFullPhone(account.presentation.phoneNumber)" :title="t('可重新授权或导入当前登录账号以更新手机号', 'Sign in again or import the current login to update the phone')">{{ t('（官方已脱敏）', '(masked by source)') }}</span></p>
        <div class="switch-badges">
          <span v-if="account.disabled" class="card-chip off">{{ t('已禁用', 'Disabled') }}</span>
          <a-tooltip
            :content="
              account.checkinEnabled
                ? account.checkin?.today_checked_in
                  ? t('今日已签到', 'Checked in today')
                  : t('已开启此账号签到', 'Check-in enabled')
                : t('此账号签到已关闭', 'Check-in disabled')
            "
            ><span class="badge-symbol" :class="{ muted: !account.checkinEnabled }"
              ><IconCalendar /></span
          ></a-tooltip>
          <span v-if="travel?.label === 'no-buddy'" class="no-buddy">{{ t('无 Buddy', 'No Buddy') }}</span>
          <a-tooltip v-else-if="travel && travel.label !== 'unsupported'" :content="travelTip"
            ><span class="badge-symbol" :class="{ muted: travel.label === 'untraveled' }"
              ><IconSend /></span
          ></a-tooltip>
          <a-popover v-if="limits.length" position="bottom" trigger="hover"
            ><span class="limit-chip" :aria-label="t(`${limits.length} 个模型受限`, `${limits.length} models limited`)"
              ><IconThunderbolt /><b v-if="limits.length > 1">{{ limits.length }}</b></span
            ><template #content
              ><div class="limit-popover">
                <strong>{{ t('模型限额 · 官方恢复时间', 'Model limits · official reset') }}</strong>
                <p v-for="l in limits" :key="l.model ?? 'unknown'">
                  <b>{{ l.model ?? t('未知模型', 'Unknown model') }}</b
                  ><span>{{ remaining(l.resetAt, now) }}（{{ expiry(l.resetAt) }}）</span>
                </p>
              </div></template
            ></a-popover
          >
          <a-tooltip
            v-if="(account.credit?.expiringSoonRemaining ?? 0) > 0"
            :content="t(`${credits(account.credit?.expiringSoonRemaining)} 积分将在 7 天内到期`, `${credits(account.credit?.expiringSoonRemaining)} credits expire within 7 days`)"
            ><span class="limit-chip"><IconStar /></span
          ></a-tooltip>
          <button
            v-if="loginInvalid"
            class="auth-invalid-chip"
            type="button"
            :aria-label="t('重新登录 WorkBuddy 账号', 'Sign in to WorkBuddy again')"
            @click="emit('relogin')"
            >{{ expired ? t('登录已过期，重新登录', 'Expired · sign in') : t('需重新登录', 'Sign-in required') }}</button
          >
        </div>
      </div>
      <a-checkbox class="account-select" :model-value="selected" :aria-label="`${t('选择账号', 'Select account')} ${name}`" @change="checked => emit('select', !!checked)" />
      <a-dropdown trigger="click" position="br" :popup-max-height="false"
        ><button class="card-more" :aria-label="t('账号操作', 'Account actions')"><IconMore /></button
        ><template #content
          ><a-doption @click="emit('info')"><IconInfoCircle /> {{ t('账号信息', 'Account info') }}</a-doption
          ><a-doption @click="emit('refresh')"><IconRefresh /> {{ t('刷新积分', 'Refresh credits') }}</a-doption
          ><a-doption @click="emit('checkin')" :disabled="account.variant !== 'cn'"
            ><IconCalendar /> {{ t('查询签到状态', 'Check-in status') }}</a-doption
          ><a-doption @click="emit('json')"><IconCode /> {{ t('查看 JSON', 'View JSON') }}</a-doption
          ><a-doption @click="setAccountDisabled('workbuddy', account.id, !account.disabled)"
            ><template v-if="account.disabled"><IconPlayCircle /> {{ t('启用账号', 'Enable account') }}</template
            ><span v-else class="danger-option"><IconStop /> {{ t('禁用账号', 'Disable account') }}</span></a-doption
          ><a-doption :disabled="busy" @click="emit('remove')"><span class="danger-option">{{ t('删除账号', 'Delete account') }}</span></a-doption
          ></template
        ></a-dropdown
      >
      <div v-if="targets.some(target => active[target.id])" class="wb-current-clients">
        <span v-for="target in targets.filter(target => active[target.id])" :key="target.id"><IconCheck /> {{ target.name }}</span>
      </div>
    </header>
    <AccountDisabledBar v-if="account.disabled" provider="workbuddy" :id="account.id" />
    <AccountAuthAlert v-else-if="loginInvalid" :expired="expired" :disabled="!account.enabled" />
    <section class="switch-credit-section">
      <div class="switch-credit-line">
        <IconStar /><strong>{{
          account.credit?.ok
            ? credits(account.credit.totalRemaining)
            : creditLoading
              ? t('查询中', 'Loading')
              : '—'
        }}</strong
        ><span>{{ account.credit?.ok ? (loginInvalid ? t('缓存积分 · ', 'Cached credits · ') : '') + t(`${rows.length} 个积分包`, `${rows.length} packs`) : t('尚未查询', 'Not queried') }}</span
        ><small :class="{ stale: account.credit?.stale || loginInvalid }" :title="account.credit?.error"
          ><IconClockCircle /> {{ creditLoading ? t('刷新中 · ', 'Refreshing · ') : ''
          }}{{ account.credit?.stale || loginInvalid ? t('旧数据 · ', 'Stale · ') : ''
          }}{{ t(`${updated(account.credit?.updatedAt)} 更新`, `Updated ${updated(account.credit?.updatedAt)}`) }}</small
        >
      </div>
      <p v-if="account.credit?.ok === false" class="credit-error">
        {{ account.credit.error || t('积分查询失败', 'Credit query failed') }}
      </p>
      <div class="expiry-heading">
        <span>{{ t('近期到期', 'Expiring soon') }}</span
        ><button :disabled="!account.credit?.ok" @click="emit('resources')">
          {{ t('查看全部积分包', 'All packs') }} <IconArrowRight />
        </button>
      </div>
      <div
        v-for="(r, index) in rows.slice(0, 2)"
        :key="`${r.packageCode}-${index}`"
        class="switch-package"
      >
        <div>
          <b>{{ credits(r.remaining) }} {{ t('积分', 'cr') }}</b
          ><span :title="resourceName(r)">{{ resourceName(r) }}</span
          ><time :class="{ near: near(r.expireAt) }">{{ expiry(r.expireAt) }}</time>
        </div>
        <div class="package-track">
          <i
            :class="{ near: near(r.expireAt) }"
            :style="{
              width: `${Math.min(100, Math.max(0, r.total > 0 ? (r.remaining / r.total) * 100 : 0))}%`,
            }"
          ></i>
        </div>
      </div>
      <div
        v-for="slot in Math.max(0, 2 - rows.length)"
        :key="`empty-${slot}`"
        class="empty-package"
      >
        {{
          slot === 1 ? (account.credit?.ok ? t('暂无其他积分包', 'No other packs') : t('刷新积分后显示到期信息', 'Refresh credits to see expiry')) : '\u00a0'
        }}
      </div>
      <div v-if="limits.length" class="inline-limits">
        <span
          v-for="l in limits"
          :key="l.model ?? 'unknown'"
          :title="t(`官方恢复时刻 ${expiry(l.resetAt)}`, `Official reset ${expiry(l.resetAt)}`)"
          ><IconThunderbolt /> {{ l.model ?? t('未知模型', 'Unknown model') }} · {{ remaining(l.resetAt, now) }}</span
        >
      </div>
    </section>
    <footer class="switch-card-footer">
      <div class="wb-client-heading"><span>{{ t('客户端账号', 'Client accounts') }}</span><button :disabled="busy" :title="t('手动续期登录 Token', 'Renew the login token now')" @click="emit('renew')"><IconRefresh :spin="busy" /> {{ t('续期', 'Renew') }}</button></div>
      <div class="wb-client-grid">
        <button v-for="target in targets" :key="target.id" type="button"
          :disabled="busy || (!target.available && !active[target.id])"
          :class="[{ selected: active[target.id] }, target.id]"
          :aria-label="t(`切换 ${target.name} 账号`, `Switch ${target.name} account`)"
          :title="target.available || active[target.id] ? t(`切换 ${target.name} 账号及会话`, `Switch ${target.name} account and sessions`) : t(`未配置 ${target.name} 客户端或扩展`, `${target.name} client not configured`)"
          @click="emit('switch', target.id)">
          <ProviderMark v-if="target.id === 'workbuddy'" provider="workbuddy" />
          <i v-else>{{ target.mark }}</i>
          <span><b>{{ target.name }}</b><small>{{ active[target.id] ? t('当前账号', 'Signed in') : target.available ? t('切换账号', 'Switch account') : t('未配置', 'Not configured') }}</small></span>
          <IconCheck v-if="active[target.id]" class="wb-client-check" />
        </button>
      </div>
      <div class="account-toggle-row">
        <label
          :title="
            account.disabled
              ? t('账号已禁用，启用后才能开启反代', 'Enable the account to turn the proxy on')
              : t('停用后不再接收新请求，正在进行的请求会继续完成', 'Disabled accounts take no new requests; in-flight requests finish')
          "
          >{{ t('反代', 'Proxy') }}
          <a-switch
            size="small"
            :model-value="account.enabled"
            :disabled="account.disabled"
            :loading="busy"
            @change="(v) => emit('enable', !!v)" /></label
        ><label
          :title="
            account.disabled
              ? t('账号已禁用，不参与签到', 'Disabled accounts are not checked in')
              : account.variant === 'cn'
                ? t('自动签到需要全局开关也开启', 'Auto check-in also needs the global switch')
                : t('国际版不支持签到', 'Not available for international')
          "
          >{{ t('签到', 'Check-in') }}
          <a-switch
            size="small"
            :model-value="account.checkinEnabled"
            :disabled="account.variant !== 'cn' || account.disabled"
            :loading="busy"
            @change="(v) => emit('checkinEnable', !!v)" /></label
        ><span :title="t(`后台管理查询 ${account.managementInFlight ?? 0} 个，不占用聊天并发`, `${account.managementInFlight ?? 0} management queries (separate from chat)`)"
          >{{ account.inFlight }} / {{ account.concurrency ?? 4 }} {{ t('并发', 'active') }}</span
        >
      </div>
    </footer>
  </article>
</template>


<style scoped>
.wb-account-card .switch-card-header { padding-bottom: 38px; }
.wb-current-clients { position: absolute; left: 17px; right: 17px; bottom: 11px; display: flex; flex-wrap: wrap; gap: 8px; }
.wb-current-clients span { display: inline-flex; align-items: center; gap: 4px; color: var(--accent); font-size: 10px; font-weight: 600; }
.wb-client-heading { display: flex; justify-content: space-between; align-items: center; margin-bottom: 10px; color: var(--muted); font-size: 10px; }
.wb-client-heading button { display: inline-flex; align-items: center; gap: 4px; border: 0; background: transparent; color: var(--muted); padding: 3px 0 3px 6px; cursor: pointer; font-size: 11px; }
.wb-client-grid { display: grid; grid-template-columns: repeat(2, minmax(0, 1fr)); gap: 7px; }
.wb-client-grid > button { position: relative; display: flex; align-items: center; gap: 8px; min-height: 48px; padding: 8px 9px; border: 1px solid var(--line); border-radius: 9px; color: var(--text); background: var(--surface); text-align: left; cursor: pointer; transition: background .15s, border-color .15s; }
.wb-client-grid > button:hover:enabled { border-color: var(--accent); background: color-mix(in srgb, var(--accent) 4%, var(--surface)); }
.wb-client-grid > button:focus-visible, .wb-client-heading button:focus-visible { outline: 2px solid var(--accent); outline-offset: 2px; }
.wb-client-grid > button:disabled { opacity: .48; cursor: default; }
.wb-client-grid > button.selected { border-color: color-mix(in srgb, var(--accent) 35%, var(--line)); background: color-mix(in srgb, var(--accent) 6%, var(--surface)); }
.wb-client-grid i, .wb-client-grid :deep(.pmark) { width: 25px; height: 25px; border-radius: 7px; flex: none; }
.wb-client-grid i { display: grid; place-items: center; font: 700 13px monospace; color: white; background: #9477d4; }
.wb-client-grid .vscode i { background: #448dc3; }
.wb-client-grid .cli i { background: #303c38; }
.wb-client-grid b { display: block; font-size: 11px; font-weight: 600; }
.wb-client-grid small { display: block; font-size: 9px; margin-top: 3px; color: var(--muted); }
.wb-client-check { margin-left: auto; flex: none; color: var(--accent); font-size: 13px; }
.compact-card .wb-client-grid > button { min-height: 40px; padding: 6px 8px; }
.compact-card .wb-client-grid small { display: none; }
</style>
