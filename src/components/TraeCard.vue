<script setup lang="ts">
import ProviderMark from './ProviderMark.vue'
import { computed } from 'vue'
import { IconMore, IconRefresh, IconSwap, IconCalendar } from '@arco-design/web-vue/es/icon'
import { t, fmtNumber, fmtShort, fmtTime, fmtDate } from '../i18n'
import { state } from '../store'
import ProxyState from './ProxyState.vue'
const props = defineProps<{ account: any; compact?: boolean }>()
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
}>()
const a = computed(() => props.account)
const name = computed(() => a.value.note || a.value.username || 'Trae')
const b = computed(() => a.value.balance || {})
const usd = computed(() => b.value.unit === 'usd')
const packs = computed(() => (a.value.creditPacks || []).filter((p: any) => p.active))
const pct = computed(() =>
  b.value.total ? Math.max(0, Math.min(100, (b.value.remaining / b.value.total) * 100)) : 0
)
const near = (p: any) => p.expiresAt && Date.parse(p.expiresAt) - state.now < 7 * 864e5
const busy = computed(() => (state.pool?.trae?.busy || []).includes(a.value.id))
</script>
<template>
  <article class="acard">
    <div class="acard-top">
      <div class="avatar trae">{{ (name || 'T').slice(0, 1).toUpperCase() }}</div>
      <div class="acard-id">
        <strong :title="name">{{ name }}</strong>
        <small
          >{{ a.email || t('本地采集账号', 'Locally captured') }} ·
          {{ a.snapshot?.pay?.user_pay_identity_str || a.plan }}</small
        >
        <div class="acard-badges">
          <span class="ptag"><ProviderMark provider="trae" />Trae</span>
          <span class="rtag">{{
            a.variant === 'cn' ? t('国内版', 'China') : t('国际版', 'International')
          }}</span>
          <span class="state-pill" :class="{ bad: a.tokenExpired }">{{
            a.tokenExpired ? t('凭证过期', 'Expired') : t('凭证有效', 'Valid')
          }}</span>
          <span v-if="a.models?.length" class="rtag"
            >{{ a.models.length }} {{ t('个模型', 'models') }}</span
          >
        </div>
      </div>
      <a-dropdown trigger="click">
        <a-button size="mini" type="text" :aria-label="t('账号操作', 'Account actions')"
          ><IconMore
        /></a-button>
        <template #content>
          <a-doption @click="emit('note')">{{ t('修改备注', 'Edit note') }}</a-doption>
          <a-doption @click="emit('token')">{{ t('刷新登录凭证', 'Refresh login') }}</a-doption>
          <a-doption @click="emit('models')">{{ t('同步模型目录', 'Sync models') }}</a-doption>
          <a-doption v-if="a.variant === 'cn'" @click="emit('packs')">{{
            t('全部积分包', 'All credit packs')
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
        >{{ usd && b.remaining != null ? '$' : '' }}{{ fmtNumber(b.remaining)
        }}<small>{{ usd ? 'USD' : t('积分', 'credits') }}</small></strong
      >
      <small>{{
        a.snapshot
          ? t(
              `更新于 ${fmtTime(a.snapshot.time).slice(0, 5)}`,
              `Updated ${fmtTime(a.snapshot.time).slice(0, 5)}`
            )
          : t('等待刷新用量', 'Not refreshed')
      }}</small>
    </div>
    <div class="track"><i :style="{ width: pct + '%' }"></i></div>
    <template v-if="!compact && a.variant === 'cn'">
      <div class="pack-row" style="grid-template-columns: 1fr auto">
        <span
          >{{ t('近期到期', 'Expiring soon') }} · {{ packs.length }}
          {{ t('个可用积分包', 'active packs') }}</span
        >
        <a @click="emit('packs')" style="cursor: pointer; font-size: 12px"
          >{{ t('查看全部', 'View all') }} →</a
        >
      </div>
      <div v-for="(p, i) in packs.slice(0, 2)" :key="i" class="pack-row">
        <b>{{ fmtNumber(p.remaining) }}</b
        ><span>{{ p.name }}</span
        ><time :class="{ near: near(p) }">{{
          p.expiresAt ? fmtShort(p.expiresAt) : t('无到期时间', 'No expiry')
        }}</time>
      </div>
      <small v-if="!packs.length" class="muted" style="font-size: 12px">{{
        a.snapshot?.usage
          ? t('暂无可用积分包', 'No active packs')
          : t('刷新用量后显示积分包', 'Refresh usage to see packs')
      }}</small>
      <div class="pack-row" style="grid-template-columns: 1fr auto auto">
        <span
          >{{
            a.checkedToday ? t('今日已签到', 'Checked in today') : t('每日签到', 'Daily check-in')
          }}
          · {{ t('自动', 'Auto') }}</span
        >
        <a-button size="mini" :loading="busy" @click="emit('checkin')"
          ><template #icon><IconCalendar /></template
          >{{ a.checkedToday ? t('检查', 'Check') : t('签到', 'Check in') }}</a-button
        >
        <a-switch
          size="small"
          :aria-label="t('自动签到', 'Auto check-in')"
          :model-value="a.autoCheckin"
          @change="(v) => emit('autoCheckin', !!v)"
        />
      </div>
    </template>
    <small v-else-if="!compact" class="muted" style="font-size: 12px">{{
      t('国际版未提供官方签到入口', 'No official check-in for international')
    }}</small>
    <small v-if="a.snapshot?.errors?.usage" style="color: var(--warn); font-size: 12px">{{
      a.snapshot.errors.usage
    }}</small>
    <div class="acard-foot">
      <div class="ops">
        <a-button size="small" :loading="busy" @click="emit('refresh')"
          ><template #icon><IconRefresh /></template>{{ t('刷新', 'Refresh') }}</a-button
        >
        <a-button size="small" type="primary" :loading="busy" @click="emit('switch')"
          ><template #icon><IconSwap /></template>{{ t('切换', 'Switch') }}</a-button
        >
      </div>
      <ProxyState provider="trae" :id="a.id" />
    </div>
    <small class="muted" style="font-size: 11px"
      >{{ t('凭证到期', 'Login expires') }} {{ fmtDate(a.expiredAt) }}</small
    >
  </article>
</template>
