<script setup lang="ts">
// Qoder account card in the WorkBuddy card layout.
import { computed } from 'vue'
import {
  IconMore,
  IconRefresh,
  IconSwap,
  IconGift,
  IconStar,
  IconClockCircle,
  IconArrowRight,
  IconCheck,
  IconInfoCircle,
  IconEdit,
  IconLock,
  IconCode,
  IconDelete,
  IconMobile,
  IconEmail,
} from '@arco-design/web-vue/es/icon'
import { t, fmtDate } from '../i18n'
import { state, revealPhones } from '../store'
import { credits, updated, phoneText } from '../lib/account-display'
import CardProxyRow from './CardProxyRow.vue'
const props = defineProps<{ account: any; compact?: boolean }>()
const emit = defineEmits<{
  switch: []
  claim: []
  credits: []
  info: []
  note: []
  refreshToken: []
  remove: []
  json: []
}>()
const a = computed(() => props.account)
const name = computed(() => a.value.note || a.value.name || a.value.email || a.value.phone || t('未命名', 'Unnamed'))
const phone = computed(() => phoneText(a.value.phone, revealPhones.value))
const usage = computed(() => a.value.credit?.usage)
const addonExpiry = computed(() => {
  const ts = Number(usage.value?.expiresAt) || 0
  if (!ts) return '—'
  return ts >= 253402214400000 ? t('长期有效', 'No expiry') : fmtDate(ts)
})
const rows = computed(() => [
  { label: t('订阅额度', 'Subscription'), q: usage.value?.userQuota, time: '' },
  { label: t('个人资源包', 'Bonus packs'), q: usage.value?.addOnQuota, time: Number(usage.value?.addOnQuota?.total) > 0 ? addonExpiry.value : '' },
])
const known = computed(() => rows.value.some((r) => r.q && typeof r.q.remaining === 'number' && Number.isFinite(r.q.remaining)))
const total = computed(() => rows.value.reduce((s, r) => s + (Number(r.q?.remaining) || 0), 0))
const pct = (q: any) => (q && Number(q.total) > 0 ? Math.min(100, Math.max(0, (Number(q.remaining) / Number(q.total)) * 100)) : 0)
const expiry = computed(() => {
  const d = Number(a.value.expiresAt ? Date.parse(a.value.expiresAt) : NaN)
  if (!Number.isFinite(d)) return { cls: 'muted', text: t('有效期未知', 'Expiry unknown') }
  const days = (d - state.now) / 864e5
  if (days <= 0) return { cls: 'bad', text: t('登录已过期', 'Login expired') }
  if (days < 3) return { cls: 'warn', text: days < 1 ? t(`${Math.max(1, Math.ceil(days * 24))} 小时后过期`, `${Math.max(1, Math.ceil(days * 24))}h left`) : t(`${Math.ceil(days)} 天后过期`, `${Math.ceil(days)}d left`) }
  return { cls: '', text: t('有效期至 ', 'Valid to ') + fmtDate(d) }
})
const color = computed(() => Array.from(String(name.value)).reduce((n, c) => n + c.charCodeAt(0), 0) % 3)
const pending = (k: string) => state.pending.has(`${k}:${a.value.id}`)
</script>
<template>
  <article class="switch-card" :class="[{ 'current-card': a.isCurrent, 'compact-card': compact }, `tone-${color}`]">
    <header class="switch-card-header">
      <div class="switch-avatar">{{ Array.from(String(name))[0] }}</div>
      <div class="switch-identity">
        <h3 :title="name">{{ name }}</h3>
        <p :title="a.uid">UID · {{ a.uid }}</p>
        <p v-if="phone" class="switch-contact mono"><IconMobile /> {{ phone }}</p>
        <p v-if="a.email" class="switch-contact" :title="a.email"><IconEmail /> {{ a.email }}</p>
        <div class="switch-badges">
          <span class="card-chip">{{ a.edition === 'cn' ? t('国内版', 'China') : t('国际版', 'Intl') }}</span>
          <span v-if="a.isCurrent" class="card-chip ok">{{ t('当前登录', 'Signed in') }}</span>
          <span class="card-chip" :class="expiry.cls || 'ok'">{{ expiry.text }}</span>
        </div>
      </div>
      <a-dropdown trigger="click"
        ><button class="card-more" :aria-label="t('账号操作', 'Account actions')"><IconMore /></button
        ><template #content
          ><a-doption @click="emit('info')"><IconInfoCircle /> {{ t('积分详情', 'Credit details') }}</a-doption
          ><a-doption @click="emit('note')"><IconEdit /> {{ t('修改备注', 'Edit note') }}</a-doption
          ><a-doption @click="emit('refreshToken')"><IconLock /> {{ t('刷新登录凭证', 'Refresh login') }}</a-doption
          ><a-doption @click="emit('json')"><IconCode /> {{ t('查看 JSON', 'View JSON') }}</a-doption
          ><a-doption @click="emit('remove')"
            ><span class="danger-option"><IconDelete /> {{ t('删除账号', 'Delete account') }}</span></a-doption
          ></template
        ></a-dropdown
      >
      <span v-if="a.isCurrent" class="client-watermark">Q</span>
    </header>
    <section class="switch-credit-section">
      <div class="switch-credit-line">
        <IconStar /><strong>{{ known ? credits(total) : pending('qcredit') ? t('查询中', 'Loading') : '—' }}</strong
        ><span>{{ t('剩余积分', 'credits left') }}</span
        ><small :class="{ stale: a.credit?.stale }" :title="a.credit?.error"
          ><IconClockCircle /> {{ a.credit?.stale ? t('旧数据 · ', 'Stale · ') : a.credit?.cached ? t('缓存 · ', 'Cached · ') : '' }}{{ a.credit?.updatedAt ? t(`${updated(a.credit.updatedAt)} 更新`, `Updated ${updated(a.credit.updatedAt)}`) : t('尚未查询', 'Not queried') }}</small
        >
      </div>
      <p v-if="a.credit?.ok === false || a.credit?.stale" class="credit-error">
        {{ a.credit?.error || t('查询失败，显示旧数据', 'Query failed; showing cached data') }}
      </p>
      <div class="expiry-heading">
        <span>{{ t('额度明细', 'Quota') }}</span
        ><button :disabled="!a.credit" @click="emit('info')">{{ t('积分详情', 'Details') }} <IconArrowRight /></button>
      </div>
      <div v-for="r in rows" :key="r.label" class="switch-package">
        <div>
          <b>{{ r.q && r.q.remaining != null ? `${credits(Number(r.q.remaining))} / ${credits(Number(r.q.total || 0))}` : t('待查询', 'Pending') }}</b
          ><span>{{ r.label }}</span><time>{{ r.time }}</time>
        </div>
        <div class="package-track"><i :style="{ width: `${pct(r.q)}%` }"></i></div>
      </div>
    </section>
    <footer class="switch-card-footer">
      <div class="client-buttons">
        <button class="qoder" :class="{ selected: a.isCurrent }" :disabled="a.isCurrent || pending('qswitch')" @click="emit('switch')">
          <i><IconSwap /></i><IconCheck v-if="a.isCurrent" /><span>{{ a.isCurrent ? t('当前账号', 'Current') : t('切换', 'Switch') }}</span>
        </button>
        <button class="claim" :disabled="pending('qclaim')" @click="emit('claim')"><i><IconGift /></i><span>{{ t('领取', 'Claim') }}</span></button>
        <button class="refresh" :disabled="pending('qcredit')" @click="emit('credits')"><i><IconRefresh /></i><span>{{ t('刷新积分', 'Credits') }}</span></button>
      </div>
      <CardProxyRow provider="qoder" :id="a.id" />
    </footer>
  </article>
</template>
