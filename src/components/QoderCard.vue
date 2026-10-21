<script setup lang="ts">
import ProviderMark from './ProviderMark.vue'
import { computed, ref } from 'vue'
import { IconMore, IconRefresh, IconSwap, IconGift } from '@arco-design/web-vue/es/icon'
import { t, fmtNumber, fmtTime, fmtDate } from '../i18n'
import { state } from '../store'
import ProxyState from './ProxyState.vue'
const props = defineProps<{ account: any; compact?: boolean }>()
const emit = defineEmits<{
  switch: []
  claim: []
  credits: []
  info: []
  note: []
  refreshToken: []
  remove: []
}>()
const reveal = ref(false)
const a = computed(() => props.account)
const name = computed(
  () => a.value.note || a.value.name || a.value.email || a.value.phone || t('未命名', 'Unnamed')
)
const contact = computed(() => {
  if (a.value.email) return a.value.email
  const p = String(a.value.phone || '')
  if (!p) return ''
  return reveal.value
    ? p
    : p.length >= 8
      ? p.slice(0, 3) + '****' + p.slice(-4)
      : p.replace(/./g, '*')
})
const usage = computed(() => a.value.credit?.usage)
const parts = computed(() => [
  [t('订阅额度', 'Subscription'), usage.value?.userQuota],
  [t('个人资源包', 'Bonus packs'), usage.value?.addOnQuota],
])
const known = computed(() =>
  parts.value.some(([, q]: any) => q && Number.isFinite(Number(q.remaining)))
)
const total = computed(() =>
  parts.value.reduce((s, [, q]: any) => s + (Number(q?.remaining) || 0), 0)
)
const expiry = computed(() => {
  const d = Number(a.value.expiresAt ? Date.parse(a.value.expiresAt) : NaN)
  if (!Number.isFinite(d)) return { cls: 'off', text: t('有效期未知', 'Expiry unknown') }
  const days = (d - state.now) / 864e5
  if (days <= 0) return { cls: 'bad', text: t('token 已过期', 'Token expired') }
  if (days < 1)
    return {
      cls: 'warn',
      text: t(
        `${Math.max(1, Math.ceil(days * 24))} 小时后过期`,
        `${Math.max(1, Math.ceil(days * 24))}h to expiry`
      ),
    }
  if (days < 3)
    return { cls: 'warn', text: t(`${Math.ceil(days)} 天后过期`, `${Math.ceil(days)}d to expiry`) }
  return { cls: '', text: t('有效期至 ', 'Valid to ') + fmtDate(d) }
})
const addonExpiry = computed(() => {
  const ts = Number(usage.value?.expiresAt) || 0
  if (!ts || ts >= 253402214400000) return t('长期有效', 'Never expires')
  return fmtDate(ts)
})
</script>
<template>
  <article class="acard" :class="{ current: a.isCurrent }">
    <div class="acard-top">
      <div class="avatar qoder">{{ Array.from(name)[0] }}</div>
      <div class="acard-id">
        <strong :title="name">{{ name }}</strong>
        <small :title="a.uid">UID · {{ a.uid }}</small>
        <small v-if="contact"
          >{{ contact }}
          <a
            v-if="a.phone && !a.email"
            style="cursor: pointer; margin-left: 4px"
            @click="reveal = !reveal"
            >{{ reveal ? t('隐藏', 'hide') : t('显示', 'show') }}</a
          ></small
        >
        <div class="acard-badges">
          <span class="ptag"><ProviderMark provider="qoder" />Qoder</span>
          <span class="rtag">{{
            a.edition === 'cn' ? t('国内版', 'China') : t('国际版', 'International')
          }}</span>
          <span v-if="a.isCurrent" class="state-pill">{{ t('当前登录', 'Signed in') }}</span>
          <span class="state-pill" :class="expiry.cls">{{ expiry.text }}</span>
        </div>
      </div>
      <a-dropdown trigger="click">
        <a-button size="mini" type="text" :aria-label="t('账号操作', 'Account actions')"
          ><IconMore
        /></a-button>
        <template #content>
          <a-doption @click="emit('info')">{{ t('积分详情', 'Credit details') }}</a-doption>
          <a-doption @click="emit('note')">{{ t('修改备注', 'Edit note') }}</a-doption>
          <a-doption @click="emit('refreshToken')">{{
            t('刷新登录凭证', 'Refresh login')
          }}</a-doption>
          <a-doption @click="emit('remove')"
            ><span style="color: var(--danger)">{{
              t('删除账号', 'Delete account')
            }}</span></a-doption
          >
        </template>
      </a-dropdown>
    </div>
    <div class="balance">
      <strong
        >{{ known ? fmtNumber(total) : '—'
        }}<small>{{ t('剩余积分', 'credits left') }}</small></strong
      >
      <small>{{
        a.credit?.updatedAt
          ? t(
              `${fmtTime(a.credit.updatedAt).slice(0, 5)} 更新`,
              `Updated ${fmtTime(a.credit.updatedAt).slice(0, 5)}`
            )
          : t('尚未查询', 'Not queried')
      }}</small>
    </div>
    <template v-if="!compact">
      <div v-for="([label, q], i) in parts" :key="i">
        <div class="pack-row" style="grid-template-columns: 1fr auto; margin-bottom: 4px">
          <span>{{ label }}</span>
          <b>{{
            q && q.remaining != null
              ? fmtNumber(q.remaining) + ' / ' + fmtNumber(q.total || 0)
              : t('待查询', 'Pending')
          }}</b>
        </div>
        <div class="track">
          <i
            :style="{
              width: `${q && Number(q.total) > 0 ? Math.min(100, (Number(q.remaining) / Number(q.total)) * 100) : 0}%`,
            }"
          ></i>
        </div>
      </div>
      <small v-if="Number(usage?.addOnQuota?.total) > 0" class="muted" style="font-size: 12px"
        >{{ t('资源包有效期至 ', 'Packs valid to ') }}{{ addonExpiry }}</small
      >
      <small
        v-if="a.credit?.stale || a.credit?.ok === false"
        style="color: var(--warn); font-size: 12px"
        >{{
          a.credit?.error || t('查询失败，显示旧数据', 'Query failed; showing cached data')
        }}</small
      >
    </template>
    <div class="acard-foot">
      <div class="ops">
        <a-button
          size="small"
          type="primary"
          :disabled="a.isCurrent"
          :loading="state.pending.has('qswitch:' + a.id)"
          @click="emit('switch')"
          ><template #icon><IconSwap /></template
          >{{ a.isCurrent ? t('当前账号', 'Current') : t('切换', 'Switch') }}</a-button
        >
        <a-button size="small" :loading="state.pending.has('qclaim:' + a.id)" @click="emit('claim')"
          ><template #icon><IconGift /></template>{{ t('领取', 'Claim') }}</a-button
        >
        <a-button
          size="small"
          :loading="state.pending.has('qcredit:' + a.id)"
          @click="emit('credits')"
          ><template #icon><IconRefresh /></template>{{ t('积分', 'Credits') }}</a-button
        >
      </div>
      <ProxyState provider="qoder" :id="a.id" />
    </div>
  </article>
</template>
