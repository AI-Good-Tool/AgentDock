<script setup lang="ts">
import { t } from '../../i18n'
import { computed, ref } from 'vue'
import { exact, compact } from '../../lib/statistics'
const props = defineProps<{
  points: Array<{ date: string; value: number; segments?: number[]; records?: number }>
  token?: boolean
}>()
const emit = defineEmits<{ select: [string] }>()
const hovered = ref(-1)
const max = computed(() => Math.max(1, ...props.points.map((p) => p.value)))
const maxRequests = computed(() => Math.max(1, ...props.points.map((p) => p.records ?? 0)))
const step = computed(() => 720 / Math.max(1, props.points.length))
const colors = ['#59bc98', '#80bfd0', '#a199dd', '#e4b775']
const line = computed(() =>
  props.points
    .map(
      (p, i) =>
        `${45 + step.value * (i + 0.5)},${180 - ((p.records ?? 0) / maxRequests.value) * 150}`
    )
    .join(' ')
)
function rects(values: number[]) {
  let bottom = 180
  return values.map((v, i) => {
    const height = (v / max.value) * 150
    bottom -= height
    return { y: bottom, height, color: colors[i] }
  })
}
</script>
<template>
  <div class="stats-trend">
    <div class="stats-chart-readout">
      {{
        hovered >= 0 && points[hovered]
          ? `${points[hovered]!.date} · ${exact(points[hovered]!.value)} ${token ? 'Tokens' : t('积分', 'credits')}${token ? ` · ${exact(points[hovered]!.records ?? 0)} ${t('次请求', 'requests')}` : ''}`
          : t('悬停查看数值，点击日期筛选明细', 'Hover for values; click a day to filter details')
      }}
    </div>
    <svg
      v-if="points.length"
      viewBox="0 0 800 220"
      role="img"
      :aria-label="token ? t('每日 Token 用量与请求次数', 'Daily tokens and requests') : t('每日积分消耗趋势', 'Daily credit usage')"
    >
      <g v-for="n in 4" :key="n">
        <line x1="45" x2="765" :y1="180 - (n - 1) * 50" :y2="180 - (n - 1) * 50" stroke="#e9eeec" />
        <text x="38" :y="184 - (n - 1) * 50" text-anchor="end">
          {{ token ? compact((max * (n - 1)) / 3) : exact((max * (n - 1)) / 3) }}
        </text>
      </g>
      <g
        v-for="(p, i) in points"
        :key="p.date"
        tabindex="0"
        role="button"
        :aria-label="`${p.date}: ${exact(p.value)}`"
        @mouseenter="hovered = i"
        @mouseleave="hovered = -1"
        @focus="hovered = i"
        @blur="hovered = -1"
        @click="emit('select', p.date)"
        @keydown.enter="emit('select', p.date)"
      >
        <rect :x="45 + step * i" y="25" :width="step" height="160" fill="transparent" />
        <rect
          v-for="(r, j) in rects(p.segments ?? [p.value])"
          :key="j"
          :x="45 + step * i + step * 0.2"
          :y="r.y"
          :width="Math.max(1, step * 0.6)"
          :height="r.height"
          :fill="r.color"
          rx="2"
        />
        <text
          v-if="i % Math.max(1, Math.ceil(points.length / 8)) === 0 || i === points.length - 1"
          :x="45 + step * (i + 0.5)"
          y="202"
          text-anchor="middle"
        >
          {{ p.date.slice(5) }}
        </text>
        <title>
          {{ p.date }} · {{ exact(p.value) }} {{ token ? 'Tokens' : t('积分', 'credits')
          }}{{ token ? ` · ${p.records ?? 0} ${t('次请求', 'requests')}` : '' }}
        </title>
      </g>
      <polyline
        v-if="token"
        :points="line"
        fill="none"
        stroke="#818ca7"
        stroke-width="2"
        stroke-dasharray="4 3"
        pointer-events="none"
      />
    </svg>
    <a-empty v-else :description="t('当前范围暂无统计数据', 'No data in this range')" />
    <div v-if="token" class="stats-chart-legend">
      <span v-for="(label, i) in [t('缓存读取', 'Cache read'), t('新增输入', 'New input'), t('输出', 'Output'), t('缓存写入', 'Cache write')]" :key="label"
        ><i :style="{ background: colors[i] }" />{{ label }}</span
      ><span>{{ t('虚线 · 请求次数（独立比例）', 'Dashed · requests (separate scale)') }}</span>
    </div>
  </div>
</template>
<style scoped>
.stats-trend svg {
  width: 100%;
  max-height: 310px;
  overflow: visible;
}
.stats-trend text {
  font: 10px sans-serif;
  fill: var(--c-8a9992);
}
.stats-trend g[role='button'] {
  cursor: pointer;
}
.stats-trend g[role='button']:hover rect:not([fill='transparent']) {
  opacity: 0.75;
}
.stats-chart-readout {
  font-size: 12px;
  color: var(--c-7b8b83);
  min-height: 26px;
}
.stats-chart-legend {
  display: flex;
  gap: 18px;
  flex-wrap: wrap;
  font-size: 11px;
  color: var(--c-84948c);
}
.stats-chart-legend span {
  display: flex;
  align-items: center;
  gap: 6px;
}
.stats-chart-legend i {
  height: 8px;
  width: 8px;
  border-radius: 2px;
}
</style>
