<script setup lang="ts">
import { computed } from 'vue'
const props = defineProps<{ data: Array<{ key: string; value: number; tip?: string }>; height?: number }>()
const max = computed(() => Math.max(0.0001, ...props.data.map((d) => d.value)))
const every = computed(() => Math.max(1, Math.ceil(props.data.length / 12)))
</script>
<template>
  <div class="bars" :style="{ height: (height ?? 150) + 'px' }">
    <div v-for="(d, i) in data" :key="d.key" :title="d.tip ?? `${d.key}: ${d.value}`">
      <i :style="{ height: `${Math.max(d.value > 0 ? 3 : 1, (d.value / max) * 100)}%`, opacity: d.value > 0 ? 1 : 0.25 }"></i>
      <small>{{ i % every === 0 ? d.key.slice(5) : ' ' }}</small>
    </div>
  </div>
</template>
