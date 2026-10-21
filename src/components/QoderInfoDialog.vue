<script setup lang="ts">
import { ref, onMounted } from 'vue'
import { call } from '../api'
import { t, fmtNumber, fmtDate } from '../i18n'
import { refreshPool } from '../store'
const props = defineProps<{ account: any }>()
const emit = defineEmits<{ close: [] }>()
const data = ref<any>(null)
const error = ref('')
onMounted(async () => {
  try {
    const r = await call('qoder_credits', { id: props.account.id })
    if (!r.ok) throw new Error(r.message)
    data.value = r.data
    refreshPool()
  } catch (e) {
    error.value = String(e instanceof Error ? e.message : e)
  }
})
const status: Record<string, [string, string]> = {
  CLAIMED: ['已领取', 'Claimed'],
  CLAIMABLE: ['可领取', 'Claimable'],
  ACHIEVEMENT_NOT_COMPLETED: ['任务未完成', 'Not completed'],
}
function campName(c: any) {
  const b = c.benefit || {}
  return String(c.title || c.name || c.displayName || b.name || b.title || b.displayName || b.description || c.description || c.campaignId || '')
}
function expiry(u: any) {
  const ts = Number(u?.expiresAt) || 0
  return !ts || ts >= 253402214400000 ? t('长期有效', 'Never expires') : fmtDate(ts)
}
</script>
<template>
  <a-modal :visible="true" :title="t('用量明细 · ', 'Usage · ') + (account.note || account.name || account.uid)" :footer="false" :width="600" @cancel="emit('close')">
    <a-spin v-if="!data && !error" style="width: 100%" />
    <a-alert v-if="error" type="error">{{ error }}</a-alert>
    <template v-if="data">
      <template v-if="data.usage">
        <div
          v-for="[name, desc, q] in [
            [t('订阅版本的资源', 'Subscription quota'), t('当前订阅计划内的额度。', 'Quota included in the current plan.'), data.usage.userQuota],
            [t('个人资源包', 'Personal bonus packs'), t('通过活动或购买获得，仅供个人使用。', 'Earned via campaigns or purchase; personal use only.'), data.usage.addOnQuota],
          ]"
          :key="name"
          class="box"
          style="padding: 14px 16px; margin-bottom: 10px"
        >
          <div class="box-head" style="margin-bottom: 8px">
            <div>
              <b>{{ name }}</b>
              <p>{{ desc }}</p>
            </div>
            <span>{{ t('剩余 ', 'Remaining ') }}<b>{{ fmtNumber(q?.remaining) }}</b></span>
          </div>
          <div class="track"><i :style="{ width: `${Number(q?.total) > 0 ? Math.min(100, (Number(q.used) / Number(q.total)) * 100) : 0}%` }"></i></div>
          <p class="hint">{{ fmtNumber(q?.used) }} / {{ fmtNumber(q?.total) }} {{ q?.unit || 'credits' }}</p>
        </div>
        <p v-if="Number(data.usage.addOnQuota?.total) > 0" class="hint">
          {{ t('获赠资源包有效期至 ', 'Bonus packs valid to ') }}{{ expiry(data.usage) }}
        </p>
      </template>
      <p v-if="data.summary" class="hint">
        {{ t('累计消耗 ', 'Total used ') }}<b>{{ fmtNumber(data.summary.totalCredits) }}</b> Credits（{{ t('单日峰值 ', 'daily peak ') }}{{ fmtNumber(data.summary.peakCredits) }} / {{ data.summary.peakDate || '-' }}）
      </p>
      <div v-if="data.campaigns?.length" style="margin-top: 12px">
        <b>{{ t('最近活动', 'Recent campaigns') }}</b>
        <div v-for="c in data.campaigns" :key="c.campaignId" class="kv">
          <span>{{ campName(c).slice(0, 40) }}</span>
          <b>
            <span class="state-pill" :class="{ warn: c.claimStatus === 'CLAIMABLE', off: c.claimStatus !== 'CLAIMED' && c.claimStatus !== 'CLAIMABLE' }">{{ status[c.claimStatus] ? t(...status[c.claimStatus]) : c.claimStatus || '—' }}</span>
            <span v-if="Number(c.benefit?.amount) > 0"> +{{ fmtNumber(c.benefit.amount) }}</span>
          </b>
        </div>
      </div>
      <a-empty v-if="!data.usage && !data.summary && !data.campaigns?.length" :description="t('暂无用量数据', 'No usage data')" />
    </template>
  </a-modal>
</template>
