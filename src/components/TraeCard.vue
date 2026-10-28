<script setup lang="ts">
// Trae account card in the WorkBuddy card layout.
import { computed } from 'vue'
import {
  IconMore,
  IconRefresh,
  IconSwap,
  IconCalendar,
  IconStar,
  IconClockCircle,
  IconArrowRight,
  IconEdit,
  IconLock,
  IconCode,
  IconDelete,
  IconStorage,
  IconGift,
  IconMobile,
  IconPlayCircle,
  IconStop,
} from '@arco-design/web-vue/es/icon'
import { t, fmtDate, fmtShort } from '../i18n'
import { state, runtimeOf, setAccountDisabled } from '../store'
import AccountAuthAlert from './AccountAuthAlert.vue'
import AccountDisabledBar from './AccountDisabledBar.vue'
import { credits, updated } from '../lib/account-display'
import CardProxyRow from './CardProxyRow.vue'
const props = defineProps<{ account: any; compact?: boolean; selected?: boolean }>()
const emit = defineEmits<{
  refresh: []
  switch: []
  checkin: []
  autoCheckin: [boolean]
  packs: []
  note: []
  token: []
  models: []
  remove: []
  select: [boolean]
  json: []
}>()
const a = computed(() => props.account)
const rt = computed(() => runtimeOf('trae', a.value.id))
const disabled = computed(() => !!rt.value?.disabled)
const expired = computed(() => a.value.tokenExpired || Date.parse(a.value.expiredAt) <= state.now)
const loginInvalid = computed(() => expired.value || a.value.tokenPresent === false || rt.value?.authFailed)
const name = computed(() => a.value.note || a.value.username || 'Trae')
const b = computed(() => a.value.balance || {})
const usd = computed(() => b.value.unit === 'usd')
const amount = (v: unknown) => (v == null ? '—' : usd.value ? `$${credits(Number(v))}` : credits(Number(v)))
const packs = computed(() => (a.value.creditPacks || []).filter((p: any) => p.active))
const near = (p: any) => p.expiresAt && Date.parse(p.expiresAt) - state.now < 7 * 864e5
const pct = (p: any) => (Number(p.total) > 0 ? Math.min(100, Math.max(0, (Number(p.remaining) / Number(p.total)) * 100)) : 0)
const busy = computed(() => (state.pool?.trae?.busy || []).includes(a.value.id))
const snapshotTime = computed(() => (a.value.snapshot?.time ? Date.parse(a.value.snapshot.time) : 0))
const error = computed(() => a.value.snapshot?.errors?.usage || a.value.snapshot?.errors?.pay || '')
const plan = computed(() => a.value.snapshot?.pay?.user_pay_identity_str || a.value.plan || '')
const color = computed(() => Array.from(String(name.value)).reduce((n, c) => n + c.charCodeAt(0), 0) % 3)
</script>
<template>
  <article
    class="switch-card"
    :class="[{ 'compact-card': compact, 'auth-invalid-card': loginInvalid && !disabled, 'account-disabled-card': disabled }, `tone-${color}`]"
  >
    <header class="switch-card-header">
      <div class="switch-avatar">{{ Array.from(String(name))[0]?.toUpperCase() }}</div>
      <div class="switch-identity">
        <h3 :title="name">{{ name }}</h3>
        <p :title="a.email || a.userId">{{ a.email || t('本地采集账号', 'Locally captured') }}{{ plan ? ' · ' + plan : '' }}</p>
        <p v-if="a.mobile" class="switch-contact mono" :title="t('Trae 官方只提供脱敏手机号', 'Trae only provides a masked number')"><IconMobile /> {{ a.mobile }}</p>
        <div class="switch-badges">
          <span v-if="disabled" class="card-chip off">{{ t('已禁用', 'Disabled') }}</span>
          <span class="card-chip">{{ a.variant === 'cn' ? t('国内版', 'China') : t('国际版', 'Intl') }}</span>
          <span :class="loginInvalid ? 'auth-invalid-chip' : 'card-chip ok'">{{ loginInvalid ? expired ? t('登录已过期', 'Login expired') : t('需重新登录', 'Sign-in required') : t('凭证有效', 'Login valid') }}</span>
          <span v-if="a.models?.length" class="card-chip">{{ a.models.length }} {{ t('个模型', 'models') }}</span>
          <a-tooltip v-if="a.variant === 'cn'" :content="a.checkedToday ? t('今日已签到', 'Checked in today') : a.autoCheckin ? t('已开启自动签到', 'Auto check-in on') : t('自动签到已关闭', 'Auto check-in off')"
            ><span class="badge-symbol" :class="{ muted: !a.checkedToday }"><IconCalendar /></span
          ></a-tooltip>
        </div>
      </div>
      <a-checkbox class="account-select" :model-value="selected" :aria-label="`${t('选择账号', 'Select account')} ${name}`" @change="checked => emit('select', !!checked)" />
      <a-dropdown trigger="click" position="br" :popup-max-height="false"
        ><button class="card-more" :aria-label="t('账号操作', 'Account actions')"><IconMore /></button
        ><template #content
          ><a-doption @click="emit('note')"><IconEdit /> {{ t('修改备注', 'Edit note') }}</a-doption
          ><a-doption @click="emit('token')"><IconLock /> {{ t('刷新登录凭证', 'Refresh login') }}</a-doption
          ><a-doption @click="emit('models')"><IconStorage /> {{ t('同步模型目录', 'Sync models') }}</a-doption
          ><a-doption v-if="a.variant === 'cn'" @click="emit('packs')"><IconGift /> {{ t('全部积分包', 'All credit packs') }}</a-doption
          ><a-doption @click="emit('json')"><IconCode /> {{ t('查看 JSON', 'View JSON') }}</a-doption
          ><a-doption @click="setAccountDisabled('trae', a.id, !disabled)"
            ><template v-if="disabled"><IconPlayCircle /> {{ t('启用账号', 'Enable account') }}</template
            ><span v-else class="danger-option"><IconStop /> {{ t('禁用账号', 'Disable account') }}</span></a-doption
          ><a-doption @click="emit('remove')"
            ><span class="danger-option"><IconDelete /> {{ t('删除账号', 'Delete account') }}</span></a-doption
          ></template
        ></a-dropdown
      >
    </header>
    <AccountDisabledBar v-if="disabled" provider="trae" :id="a.id" />
    <AccountAuthAlert v-else-if="loginInvalid" :expired="!!expired" :disabled="!(rt?.enabled ?? a.enabled)" />
    <section class="switch-credit-section">
      <div class="switch-credit-line">
        <IconStar /><strong>{{ amount(b.remaining) }}</strong
        ><span>{{ loginInvalid ? (usd ? t('缓存余额', 'Cached balance') : t('缓存积分', 'Cached credits')) : usd ? 'USD' : t('剩余积分', 'credits left') }}{{ b.total ? ` / ${amount(b.total)}` : '' }}</span
        ><small :class="{ stale: loginInvalid }" :title="a.snapshot?.time"
          ><IconClockCircle /> {{ busy ? t('刷新中 · ', 'Refreshing · ') : '' }}{{ loginInvalid ? t('旧数据 · ', 'Stale · ') : '' }}{{ snapshotTime ? t(`${updated(snapshotTime)} 更新`, `Updated ${updated(snapshotTime)}`) : t('尚未查询', 'Not queried') }}</small
        >
      </div>
      <p v-if="error" class="credit-error">{{ error }}</p>
      <template v-if="a.variant === 'cn'">
        <div class="expiry-heading">
          <span>{{ t('近期到期', 'Expiring soon') }} · {{ packs.length }} {{ t('个积分包', 'packs') }}</span
          ><button :disabled="!a.snapshot" @click="emit('packs')">{{ t('查看全部积分包', 'All packs') }} <IconArrowRight /></button>
        </div>
        <div v-for="(p, i) in packs.slice(0, 2)" :key="i" class="switch-package">
          <div>
            <b>{{ credits(Number(p.remaining)) }} {{ t('积分', 'cr') }}</b><span :title="p.name">{{ p.name }}</span
            ><time :class="{ near: near(p) }">{{ p.expiresAt ? fmtShort(p.expiresAt) : t('长期有效', 'No expiry') }}</time>
          </div>
          <div class="package-track"><i :class="{ near: near(p) }" :style="{ width: `${pct(p)}%` }"></i></div>
        </div>
        <div v-for="slot in Math.max(0, 2 - packs.length)" :key="`empty-${slot}`" class="empty-package">
          {{ slot === 1 ? (a.snapshot?.usage ? t('暂无其他积分包', 'No other packs') : t('刷新后显示积分包与到期信息', 'Refresh to see packs and expiry')) : ' ' }}
        </div>
      </template>
      <template v-else>
        <div class="expiry-heading"><span>{{ t('额度', 'Quota') }}</span></div>
        <div class="switch-package">
          <div>
            <b>{{ amount(b.used) }}</b><span>{{ t('已用', 'Used') }}</span><time>{{ t('凭证到期 ', 'Login to ') }}{{ fmtDate(a.expiredAt) }}</time>
          </div>
          <div class="package-track"><i :style="{ width: `${b.total ? Math.min(100, (b.remaining / b.total) * 100) : 0}%` }"></i></div>
        </div>
        <div class="empty-package">{{ t('国际版没有官方签到与积分包入口', 'No check-in or credit packs for international') }}</div>
      </template>
    </section>
    <footer class="switch-card-footer">
      <div class="client-buttons">
        <button class="trae" :disabled="busy" @click="emit('switch')"><i><IconSwap /></i><span>{{ t('切换', 'Switch') }}</span></button>
        <button class="refresh" :disabled="busy" @click="emit('refresh')"><i><IconRefresh /></i><span>{{ t('刷新', 'Refresh') }}</span></button>
        <button v-if="a.variant === 'cn'" class="claim" :class="{ selected: a.checkedToday }" :disabled="busy" @click="emit('checkin')">
          <i><IconCalendar /></i><span>{{ a.checkedToday ? t('今日已签到', 'Checked in') : t('签到', 'Check in') }}</span>
        </button>
      </div>
      <CardProxyRow provider="trae" :id="a.id">
        <label
          v-if="a.variant === 'cn'"
          :title="disabled ? t('账号已禁用，不参与签到', 'Disabled accounts are not checked in') : t('自动签到按「签到与领取」的计划执行', 'Auto check-in follows the Daily Rewards schedule')"
          >{{ t('签到', 'Check-in') }}
          <a-switch size="small" :model-value="a.autoCheckin" :disabled="disabled" @change="(v) => emit('autoCheckin', !!v)"
        /></label>
      </CardProxyRow>
    </footer>
  </article>
</template>
