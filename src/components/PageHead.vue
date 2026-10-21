<script setup lang="ts">
defineProps<{ eyebrow: string; title: string; desc?: string }>()
</script>
<template>
  <div class="page-head">
    <div>
      <div class="eyebrow">{{ eyebrow }}</div>
      <h1>{{ title }}</h1>
      <p v-if="desc">{{ desc }}</p>
    </div>
    <div class="actions"><slot /></div>
  </div>
</template>
