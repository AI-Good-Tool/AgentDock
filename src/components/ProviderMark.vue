<script setup lang="ts">
// Provider badge drawn on a 32×32 grid; the wrapper's size and radius decide the shape.
import { computed, useId } from 'vue'
import { providerLabel } from '../store'

const props = defineProps<{ provider: string }>()
const uid = useId()
const tile = computed(() => `${uid}-tile`)
const shine = computed(() => `${uid}-shine`)
const accent = computed(() => `${uid}-accent`)
const known = computed(() => ['workbuddy', 'trae'].includes(props.provider))
const colors: Record<string, [string, string]> = {
  workbuddy: ['#26c99c', '#0e8f70'],
  trae: ['#323c4b', '#0d1117'],
}
</script>

<template>
  <span class="pmark" :class="provider" role="img" :aria-label="providerLabel(provider)">
    <svg v-if="provider === 'qoder'" viewBox="0 0 32 32" aria-hidden="true">
      <defs>
        <linearGradient :id="tile" x1="4" y1="2" x2="28" y2="32" gradientUnits="userSpaceOnUse">
          <stop stop-color="#fff" />
          <stop offset="1" stop-color="#f1f7f3" />
        </linearGradient>
      </defs>
      <rect x=".4" y=".4" width="31.2" height="31.2" rx="8" :fill="`url(#${tile})`" stroke="#d7e2da" stroke-width=".8" />
      <rect x="4.8" y="8" width="14.8" height="9.8" rx="1.8" fill="#3f644d" />
      <rect x="12.4" y="14.2" width="14.8" height="9.8" rx="1.8" fill="#3f644d" />
      <path d="M11.8 11.6h3.6V9.8l3.7 3.7-3.7 3.7v-1.8h-3.6z" fill="#fff" />
      <path d="M23.4 18h-4.2v-1.8L15.5 20l3.7 3.7v-1.8h4.2z" fill="#fff" />
    </svg>
    <svg v-else-if="known" viewBox="0 0 32 32" aria-hidden="true">
      <defs>
        <linearGradient :id="tile" x1="0" y1="0" x2="32" y2="32" gradientUnits="userSpaceOnUse">
          <stop :stop-color="colors[provider][0]" />
          <stop offset="1" :stop-color="colors[provider][1]" />
        </linearGradient>
        <linearGradient :id="shine" x1="0" y1="0" x2="0" y2="16" gradientUnits="userSpaceOnUse">
          <stop stop-color="#fff" stop-opacity=".22" />
          <stop offset="1" stop-color="#fff" stop-opacity="0" />
        </linearGradient>
        <linearGradient :id="accent" x1="8" y1="12" x2="24" y2="20" gradientUnits="userSpaceOnUse">
          <stop stop-color="#5ff2c0" />
          <stop offset="1" stop-color="#14c98f" />
        </linearGradient>
      </defs>
      <rect width="32" height="32" :fill="`url(#${tile})`" />
      <rect width="32" height="16" :fill="`url(#${shine})`" />

      <template v-if="provider === 'workbuddy'">
        <path
          d="M7.4 12.4 11.2 22.4 16 15.4 20.8 22.4 24 13.6"
          fill="none"
          stroke="#fff"
          stroke-width="3.2"
          stroke-linecap="round"
          stroke-linejoin="round"
        />
        <circle cx="24.6" cy="8.2" r="2.3" fill="#b9f6de" />
      </template>

      <template v-else>
        <rect
          x="6.6"
          y="8.6"
          width="18.8"
          height="14.8"
          rx="2.4"
          fill="none"
          stroke="#fff"
          stroke-width="2.6"
        />
        <path
          d="M12.6 13.6 15 16l-2.4 2.4L10.2 16zM19.4 13.6 21.8 16l-2.4 2.4L17 16z"
          :fill="`url(#${accent})`"
        />
      </template>
    </svg>
    <template v-else>{{ (provider || '?')[0].toUpperCase() }}</template>
  </span>
</template>
