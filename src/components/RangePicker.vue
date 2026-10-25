<script setup lang="ts">
import { computed } from 'vue'
import { RANGE_KEYS, rangeDays, rangeLabel, type RangeKey } from '../lib/range'
import { t, dayKey } from '../i18n'
/** `today`: day key the presets count from (defaults to the display zone's). */
const props = defineProps<{ modelValue: RangeKey; custom?: [string, string] | null; today?: string }>()
const emit = defineEmits<{ 'update:modelValue': [RangeKey]; 'update:custom': [[string, string] | null] }>()
const today = computed(() => props.today ?? dayKey(Date.now()))
// The picker always shows the dates the current preset covers.
const dates = computed(() => {
  const [from, to] = rangeDays(props.modelValue, props.custom, today.value)
  return from && to ? [from, to] : undefined
})
// Calendar cells are local-midnight Dates; their label is the local calendar day, whatever the display zone.
const cellKey = (d: Date) =>
  `${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, '0')}-${String(d.getDate()).padStart(2, '0')}`
function pickPreset(v: unknown) {
  emit('update:modelValue', v as RangeKey)
  if (v !== 'custom') emit('update:custom', null)
}
function pickDates(v: unknown) {
  const range = Array.isArray(v) && v.length === 2 ? ([String(v[0]), String(v[1])] as [string, string]) : null
  emit('update:custom', range)
  emit('update:modelValue', range ? 'custom' : 'today')
}
</script>
<template>
  <div class="range-picker">
    <a-select :model-value="props.modelValue" style="width: 120px" :aria-label="t('统计时间范围', 'Range')" @change="pickPreset">
      <a-option v-for="k in RANGE_KEYS" :key="k" :value="k">{{ rangeLabel(k) }}</a-option>
      <a-option value="custom" :disabled="!props.custom">{{ rangeLabel('custom') }}</a-option>
    </a-select>
    <a-range-picker
      :model-value="dates"
      value-format="YYYY-MM-DD"
      :placeholder="[t('开始日期', 'Start'), t('结束日期', 'End')]"
      :disabled-date="(d?: Date) => !!d && cellKey(d) > today"
      style="width: 240px"
      @change="pickDates"
    />
  </div>
</template>
<style scoped>
.range-picker {
  display: flex;
  gap: 8px;
  align-items: center;
  flex-wrap: wrap;
}
</style>
