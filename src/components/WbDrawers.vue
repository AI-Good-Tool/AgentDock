<script setup lang="ts">
// WorkBuddy account info (display/note, limits) and all credit packs.
import { ref, computed, watch } from 'vue'
import { IconEye, IconEyeInvisible } from '@arco-design/web-vue/es/icon'
import { call } from '../api'
import { t } from '../i18n'
import { act, busy, state } from '../store'
import { accountName, credits, resourceRows, resourceName, expiry, remaining, phoneText, hasFullPhone } from '../lib/account-display'
const props = defineProps<{ account: any; mode: 'info' | 'resources' }>()
const emit = defineEmits<{ close: [] }>()
const note = ref('')
const field = ref('nickname')
const phoneVisible = ref(false)
watch(
  () => props.account?.id,
  () => {
    phoneVisible.value = false
    note.value = props.account?.note || props.account?.presentation?.note || ''
    field.value = props.account?.displayField || 'nickname'
  },
  { immediate: true }
)
const rows = computed(() => resourceRows(props.account?.credit?.resources, state.now))
const limits = computed(() => (props.account?.modelLimits ?? []).filter((l: any) => l.resetAt > state.now))
function save() {
  return act('wb-profile', () => call('wb_update_display', { id: props.account.id, note: note.value, field: field.value }), t('账号显示设置已保存', 'Display settings saved'))
}
function refresh() {
  return act('wb-credits', () => call('wb_refresh_credits', { ids: [props.account.id] }))
}
</script>
<template>
  <a-drawer :visible="true" :width="mode === 'info' ? 500 : 660" :footer="false" :title="mode === 'info' ? t('账号信息', 'Account info') : accountName(account) + t(' · 全部积分包', ' · All credit packs')" @cancel="emit('close')">
    <template v-if="mode === 'info'">
      <h2 class="account-info-name">{{ accountName(account) }}</h2>
      <a-descriptions :column="1" bordered>
        <a-descriptions-item label="UID">{{ account.presentation?.uid || account.id.split(':').slice(1).join(':') }}</a-descriptions-item>
        <a-descriptions-item :label="t('来源', 'Source')">{{ account.source === 'switch' ? 'WorkBuddy Switch' : t('本机登录态', 'Local login') }}</a-descriptions-item>
        <a-descriptions-item :label="t('地区', 'Region')">{{ account.variant === 'ai' ? t('国际版', 'International') : t('国内版', 'China') }}</a-descriptions-item>
        <a-descriptions-item :label="t('手机号', 'Phone')">
          <button v-if="hasFullPhone(account.presentation?.phoneNumber)" class="phone-reveal" type="button" :aria-pressed="phoneVisible" :aria-label="phoneVisible ? t('隐藏手机号', 'Hide phone number') : t('显示手机号', 'Show phone number')" @click="phoneVisible = !phoneVisible">
            {{ phoneText(account.presentation.phoneNumber, phoneVisible) }}
            <IconEyeInvisible v-if="phoneVisible" /><IconEye v-else />
          </button>
          <template v-else-if="account.presentation?.phoneNumber">{{ account.presentation.phoneNumber }} <small>{{ t('官方仅提供脱敏号码，请重新授权或导入当前登录账号', 'Only a masked number is available; sign in again or import the current login') }}</small></template>
          <template v-else>—</template>
        </a-descriptions-item>
        <a-descriptions-item :label="t('登录到期', 'Login expires')">{{ expiry(account.expiresAt) }}</a-descriptions-item>
        <a-descriptions-item :label="t('本账号签到', 'Check-in')">{{ account.checkinEnabled ? t('开启', 'On') : t('关闭', 'Off') }}</a-descriptions-item>
      </a-descriptions>
      <div class="account-info-edit">
        <label>{{ t('本地备注', 'Local note') }}</label>
        <a-input v-model="note" :placeholder="t('添加备注（最多 100 字）', 'Note (up to 100 characters)')" :max-length="100" />
        <label>{{ t('卡片显示', 'Card shows') }}</label>
        <a-radio-group v-model="field" type="button">
          <a-radio value="nickname">{{ t('账号名', 'Name') }}</a-radio>
          <a-radio value="phone">{{ t('手机号', 'Phone') }}</a-radio>
          <a-radio value="note">{{ t('备注', 'Note') }}</a-radio>
        </a-radio-group>
        <a-button type="primary" :loading="busy('wb-profile')" @click="save">{{ t('保存显示设置', 'Save') }}</a-button>
      </div>
      <div class="account-info-limits">
        <h3>{{ t('当前模型限制', 'Active model limits') }}</h3>
        <p v-for="l in limits" :key="l.model ?? 'unknown'">
          <strong>{{ l.model ?? t('未知模型', 'Unknown model') }}</strong> {{ remaining(l.resetAt, state.now) }}（{{ expiry(l.resetAt) }}）
        </p>
        <p v-if="!limits.length">{{ t('未发现仍有效的限制记录', 'No active limits') }}</p>
      </div>
    </template>
    <template v-else>
      <div class="resources-total">
        <strong>{{ credits(account.credit?.totalRemaining) }}</strong>
        <span>{{ t('剩余积分', 'credits left') }} · {{ rows.length }} {{ t('个可用积分包', 'active packs') }}</span>
        <a-button :loading="busy('wb-credits')" @click="refresh">{{ t('刷新', 'Refresh') }}</a-button>
      </div>
      <p class="resources-tip">{{ t('按到期时间升序排列；7 天内到期以橙色标记。', 'Sorted by expiry; packs expiring within 7 days are orange.') }}</p>
      <div v-for="(r, index) in rows" :key="`${r.packageCode}-${index}`" class="resource-detail-row">
        <div>
          <strong>{{ resourceName(r) }}</strong><small>{{ r.packageCode }}</small><span>{{ t('已用', 'Used') }} {{ credits(r.used) }} · {{ t('总量', 'Total') }} {{ credits(r.total) }}</span>
        </div>
        <div>
          <b>{{ credits(r.remaining) }}</b><time :class="{ near: r.expireAt && r.expireAt - state.now < 7 * 86400000 }">{{ expiry(r.expireAt) }}</time>
        </div>
        <div class="package-track">
          <i :class="{ near: r.expireAt && r.expireAt - state.now < 7 * 86400000 }" :style="{ width: `${Math.min(100, Math.max(0, r.total > 0 ? (r.remaining / r.total) * 100 : 0))}%` }"></i>
        </div>
      </div>
      <a-empty v-if="!rows.length" :description="t('暂无可用积分包', 'No active packs')" />
    </template>
  </a-drawer>
</template>

<style scoped>
.phone-reveal { display: inline-flex; align-items: center; gap: 8px; border: 0; background: transparent; color: inherit; padding: 2px 0; font: inherit; cursor: pointer; }
.phone-reveal svg { color: var(--color-text-3); }
.phone-reveal:focus-visible { outline: 2px solid rgb(var(--primary-6)); outline-offset: 3px; border-radius: 3px; }
</style>
