<script setup lang="ts">
import { computed, ref, watch } from 'vue'
import type { CheckinSchedule } from '../api'
import { checkinWeekdays, describeCheckinSchedule } from '../checkin-schedule'
import { t, zone, zoneName } from '../i18n'

const props = defineProps<{ modelValue: CheckinSchedule; disabled?: boolean; saving?: boolean }>()
const emit = defineEmits<{ save: [schedule: CheckinSchedule] }>()
const draft = ref<CheckinSchedule>(JSON.parse(JSON.stringify(props.modelValue)))
watch(() => JSON.stringify(props.modelValue), () => { draft.value = JSON.parse(JSON.stringify(props.modelValue)) })
const kinds = computed(() => [
  { value: 'minutes', label: t('按分钟间隔', 'Every N minutes') },
  { value: 'hours', label: t('按小时间隔', 'Every N hours') },
  { value: 'days', label: t('按天间隔', 'Every N days') },
  { value: 'daily', label: t('每天', 'Daily') },
  { value: 'weekly', label: t('指定星期', 'Selected weekdays') },
  { value: 'date', label: t('指定日期', 'Specific date') },
])
const intervalMode = computed(() => ['minutes', 'hours', 'days'].includes(draft.value.kind))
const maximum = computed(() => draft.value.kind === 'minutes' ? 1440 : draft.value.kind === 'hours' ? 8760 : 365)
const unit = computed(() => draft.value.kind === 'minutes' ? t('分钟', 'min') : draft.value.kind === 'hours' ? t('小时', 'h') : t('天', 'days'))
const dirty = computed(() => JSON.stringify(draft.value) !== JSON.stringify(props.modelValue))
const error = computed(() => {
  const value = draft.value
  if (intervalMode.value) return Number.isInteger(value.every) && value.every >= 1 && value.every <= maximum.value ? '' : t(`请输入 1–${maximum.value} 的整数`, `Enter an integer from 1 to ${maximum.value}`)
  if (!/^([01]\d|2[0-3]):[0-5]\d$/.test(value.time)) return t('请选择有效时间', 'Choose a valid time')
  if (value.kind === 'weekly' && !value.weekdays.length) return t('请至少选择一个星期', 'Select at least one weekday')
  if (value.kind === 'date' && !/^\d{4}-\d{2}-\d{2}$/.test(value.date)) return t('请选择执行日期', 'Choose a date')
  return ''
})
function save() {
  if (!error.value && dirty.value) emit('save', JSON.parse(JSON.stringify(draft.value)))
}
</script>

<template>
  <div class="checkin-schedule-editor" :class="{ disabled }">
    <div class="schedule-fields">
      <div class="schedule-field">
        <label for="checkin-schedule-kind">{{ t('执行方式', 'Schedule') }}</label>
        <select id="checkin-schedule-kind" v-model="draft.kind" :disabled="disabled" class="schedule-kind">
          <option v-for="kind in kinds" :key="kind.value" :value="kind.value">{{ kind.label }}</option>
        </select>
      </div>
      <div v-if="intervalMode" class="schedule-field">
        <span>{{ t('执行间隔', 'Interval') }}</span>
        <a-input-number v-model="draft.every" :min="1" :max="maximum" :precision="0" :disabled="disabled" style="width: 155px">
          <template #suffix>{{ unit }}</template>
        </a-input-number>
      </div>
      <div v-if="draft.kind === 'date'" class="schedule-field">
        <span>{{ t('执行日期', 'Date') }}</span>
        <a-date-picker v-model="draft.date" value-format="YYYY-MM-DD" :allow-clear="false" :disabled="disabled" style="width: 165px" />
      </div>
      <div v-if="!intervalMode" class="schedule-field">
        <span>{{ t('执行时间', 'Time') }}</span>
        <a-time-picker v-model="draft.time" format="HH:mm" :allow-clear="false" :disabled="disabled" style="width: 125px" />
      </div>
      <a-button type="primary" :disabled="disabled || !dirty || !!error" :loading="saving" @click="save">{{ t('保存计划', 'Save schedule') }}</a-button>
    </div>
    <div v-if="draft.kind === 'weekly'" class="schedule-weekdays">
      <span>{{ t('执行星期', 'Weekdays') }}</span>
      <a-checkbox-group v-model="draft.weekdays" :disabled="disabled">
        <a-checkbox v-for="day in checkinWeekdays()" :key="day.value" :value="day.value">{{ day.label }}</a-checkbox>
      </a-checkbox-group>
    </div>
    <p class="schedule-preview" :class="{ 'schedule-error': error }">
      {{ error || describeCheckinSchedule(draft) }}
      <template v-if="!error"> · {{ t('时区：', 'Time zone: ') }}{{ zoneName(zone) }}</template>
      <span v-if="dirty && !error"> · {{ t('尚未保存', 'Unsaved') }}</span>
    </p>
  </div>
</template>

<style scoped>
.checkin-schedule-editor { padding: 18px 20px; margin: 0 0 4px; background: var(--surface-2); border: 1px solid var(--line); border-radius: 12px; }
.schedule-fields { display: flex; align-items: end; flex-wrap: wrap; gap: 14px 18px; }
.schedule-field { display: flex; flex-direction: column; gap: 7px; color: var(--text-2); font-size: 12px; }
.schedule-kind { width: 170px; height: 32px; padding: 0 10px; border: 1px solid var(--line); border-radius: 4px; background: var(--surface); color: var(--text); font: inherit; font-size: 14px; cursor: pointer; }
.schedule-kind:focus-visible { outline: 2px solid var(--accent); outline-offset: 2px; }
.schedule-kind:disabled { cursor: default; }
.schedule-weekdays { display: flex; align-items: center; flex-wrap: wrap; gap: 12px; margin-top: 16px; color: var(--text-2); font-size: 12px; }
.schedule-preview { margin: 14px 0 0; color: var(--muted); font-size: 12px; }
.schedule-error { color: var(--danger); }
.disabled { opacity: .6; }
</style>
