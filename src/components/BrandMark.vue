<script setup lang="ts">
// AgentDock mark: three provider routes merging into one gateway port (same artwork as the app icon).
import { useId } from 'vue'
const uid = useId()
</script>
<template>
  <svg class="brand-mark" viewBox="100 100 824 824" role="img" aria-label="AgentDock">
    <defs>
      <linearGradient :id="`${uid}-bg`" x1="0.15" y1="0" x2="0.85" y2="1">
        <stop offset="0" stop-color="#23C795" />
        <stop offset="0.55" stop-color="#109A73" />
        <stop offset="1" stop-color="#075E47" />
      </linearGradient>
      <linearGradient :id="`${uid}-lane`" gradientUnits="userSpaceOnUse" x1="250" y1="0" x2="640" y2="0">
        <stop offset="0" stop-color="#FFFFFF" stop-opacity="0.82" />
        <stop offset="1" stop-color="#FFFFFF" />
      </linearGradient>
    </defs>
    <rect x="100" y="100" width="824" height="824" rx="186" :fill="`url(#${uid}-bg)`" />
    <g fill="none" stroke-linecap="round" :stroke="`url(#${uid}-lane)`" stroke-width="60">
      <path d="M280 344 C 460 344 492 512 640 512" />
      <path d="M280 512 H 640" />
      <path d="M280 680 C 460 680 492 512 640 512" />
    </g>
    <circle cx="280" cy="344" r="50" fill="#FFFFFF" />
    <circle cx="280" cy="512" r="50" fill="#B9F5DF" />
    <circle cx="280" cy="680" r="50" fill="#FFFFFF" />
    <circle cx="704" cy="512" r="116" fill="#FFFFFF" />
    <circle cx="704" cy="512" r="56" fill="#0F8A68" />
    <circle cx="704" cy="512" r="21" fill="#B9F5DF" />
  </svg>
</template>
