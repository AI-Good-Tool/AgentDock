<script setup lang="ts">
import { RANGE_KEYS, rangeLabel, type RangeKey } from '../lib/range'
import { t } from '../i18n'
const props = defineProps<{ modelValue: RangeKey; custom?: [string, string] | null }>()
const emit = defineEmits<{ 'update:modelValue': [RangeKey]; 'update:custom': [[string, string] | null] }>()
function onCustom(v: unknown) {
  const range = Array.isArray(v) && v.length === 2 ? ([String(v[0]), String(v[1])] as [string, string]) : null
  emit('update:custom', range)
  emit('update:modelValue', range ? 'custom' : 'all')
}
</script>
<template>
  <div style="display: flex; gap: 8px; align-items: center; flex-wrap: wrap">
    <a-select :model-value="props.modelValue === 'custom' ? 'custom' : props.modelValue" style="width: 130px" @change="(v) => emit('update:modelValue', v as RangeKey)">
      <a-option v-for="k in RANGE_KEYS" :key="k" :value="k">{{ rangeLabel(k) }}</a-option>
      <a-option value="custom">{{ rangeLabel('custom') }}</a-option>
    </a-select>
    <a-range-picker v-if="props.modelValue === 'custom'" :model-value="props.custom ?? undefined" value-format="YYYY-MM-DD" :placeholder="[t('开始', 'Start'), t('结束', 'End')]" style="width: 250px" @change="onCustom" />
  </div>
</template>
