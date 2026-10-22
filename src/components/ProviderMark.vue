<script setup lang="ts">
// Provider badge drawn on a 32×32 grid; the wrapper's size and radius decide the shape.
import { computed, useId } from 'vue'
import { providerLabel } from '../store'

defineProps<{ provider: string }>()
const uid = useId()
const tile = computed(() => `${uid}-tile`)
const shine = computed(() => `${uid}-shine`)
</script>

<template>
  <span class="pmark" :class="provider" role="img" :aria-label="providerLabel(provider)">
    <svg v-if="provider === 'qoder' || provider === 'trae'" viewBox="0 0 32 32" aria-hidden="true">
      <defs>
        <linearGradient :id="tile" x1="4" y1="2" x2="28" y2="32" gradientUnits="userSpaceOnUse">
          <stop stop-color="#fff" />
          <stop offset="1" stop-color="#f1f7f3" />
        </linearGradient>
      </defs>
      <rect
        x=".4"
        y=".4"
        width="31.2"
        height="31.2"
        rx="8"
        :fill="`url(#${tile})`"
        stroke="#d7e2da"
        stroke-width=".8"
      />
      <template v-if="provider === 'qoder'">
        <rect x="4.8" y="8" width="14.8" height="9.8" rx="1.8" fill="#3f644d" />
        <rect x="12.4" y="14.2" width="14.8" height="9.8" rx="1.8" fill="#3f644d" />
        <path d="M11.8 11.6h3.6V9.8l3.7 3.7-3.7 3.7v-1.8h-3.6z" fill="#fff" />
        <path d="M23.4 18h-4.2v-1.8L15.5 20l3.7 3.7v-1.8h4.2z" fill="#fff" />
      </template>
      <template v-else>
        <path d="M6.4 18V8.4h14V6l4.4 3.8-4.4 3.8v-2.4H9.2V18z" fill="#19231f" />
        <path d="M25.6 14v9.6h-14V26l-4.4-3.8 4.4-3.8v2.4h11.2V14z" fill="#00b78d" />
        <path
          d="m13.1 13.6 2.4 2.4-2.4 2.4-2.4-2.4zm5.8 0 2.4 2.4-2.4 2.4-2.4-2.4z"
          fill="#19231f"
        />
      </template>
    </svg>
    <svg v-else-if="provider === 'workbuddy'" viewBox="0 0 32 32" aria-hidden="true">
      <defs>
        <linearGradient :id="tile" x1="0" y1="0" x2="32" y2="32" gradientUnits="userSpaceOnUse">
          <stop stop-color="#26c99c" />
          <stop offset="1" stop-color="#0e8f70" />
        </linearGradient>
        <linearGradient :id="shine" x1="0" y1="0" x2="0" y2="16" gradientUnits="userSpaceOnUse">
          <stop stop-color="#fff" stop-opacity=".22" />
          <stop offset="1" stop-color="#fff" stop-opacity="0" />
        </linearGradient>
      </defs>
      <rect width="32" height="32" :fill="`url(#${tile})`" />
      <rect width="32" height="16" :fill="`url(#${shine})`" />

      <path
        d="M7.4 12.4 11.2 22.4 16 15.4 20.8 22.4 24 13.6"
        fill="none"
        stroke="#fff"
        stroke-width="3.2"
        stroke-linecap="round"
        stroke-linejoin="round"
      />
      <circle cx="24.6" cy="8.2" r="2.3" fill="#b9f6de" />
    </svg>
    <template v-else>{{ (provider || '?')[0].toUpperCase() }}</template>
  </span>
</template>
