<script setup lang="ts">
// Read-only, syntax-highlighted JSON with optional masking of secret-looking fields.
import { computed } from 'vue'

const props = defineProps<{ value: unknown; mask?: boolean }>()
const SECRET = /token|secret|password|passwd|cookie|authblob|auth_raw|apikey|api_key|credential|session_key|private/i

function masked(v: unknown, key = ''): unknown {
  if (props.mask && key && SECRET.test(key) && typeof v === 'string' && v) return v.length > 12 ? `${v.slice(0, 4)}••••••${v.slice(-4)}` : '••••••'
  if (Array.isArray(v)) return v.map((x) => masked(x))
  if (v && typeof v === 'object') return Object.fromEntries(Object.entries(v).map(([k, x]) => [k, masked(x, k)]))
  return v
}
const esc = (s: string) => s.replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;')
const html = computed(() =>
  esc(JSON.stringify(masked(props.value), null, 2) ?? 'null').replace(
    /("(?:\\.|[^"\\])*")(\s*:)?|\b(true|false)\b|\bnull\b|-?\d+(?:\.\d+)?(?:[eE][+-]?\d+)?/g,
    (m, str, colon, bool) => {
      if (str) return colon ? `<span class="j-key">${str}</span>${colon}` : `<span class="j-str">${str}</span>`
      if (bool) return `<span class="j-bool">${m}</span>`
      if (m === 'null') return `<span class="j-null">null</span>`
      return `<span class="j-num">${m}</span>`
    }
  )
)
</script>

<template>
  <!-- eslint-disable-next-line vue/no-v-html -- escaped above; only span tags are added -->
  <pre class="json-view mono" v-html="html"></pre>
</template>

<style>
.json-view {
  margin: 0;
  max-height: 56vh;
  overflow: auto;
  padding: 14px 16px;
  border: 1px solid var(--line);
  border-radius: 10px;
  background: var(--surface-2);
  color: var(--text-2);
  font-size: 12px;
  line-height: 1.65;
  white-space: pre-wrap;
  overflow-wrap: anywhere;
  tab-size: 2;
}
.json-view .j-key {
  color: #7c4dbd;
}
.json-view .j-str {
  color: #17855f;
}
.json-view .j-num {
  color: #c2620a;
}
.json-view .j-bool {
  color: #2f6fd1;
}
.json-view .j-null {
  color: var(--muted);
  font-style: italic;
}
:root[data-theme='dark'] .json-view .j-key {
  color: #c5a3ff;
}
:root[data-theme='dark'] .json-view .j-str {
  color: #6fd6a8;
}
:root[data-theme='dark'] .json-view .j-num {
  color: #f3b06b;
}
:root[data-theme='dark'] .json-view .j-bool {
  color: #7fb2ff;
}
</style>
