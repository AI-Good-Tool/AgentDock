<script setup lang="ts">
import ProviderMark from '../components/ProviderMark.vue'
import { computed } from 'vue'
import { IconCopy, IconArrowRight } from '@arco-design/web-vue/es/icon'
import { t, fmtShort, duration, fmtNumber } from '../i18n'
import { state, page, copy, providerLabel, regionLabel, PROVIDERS } from '../store'
import { outcomeName } from '../lib/request-log'
import { requestTokenUsage, tokenCount } from '../lib/request-usage'

const running = computed(() => !!state.app?.running)
const base = computed(() => `http://127.0.0.1:${state.app?.port ?? 24800}/v1`)
const runtime = computed(() => Object.values(state.pool?.runtime ?? {}) as any[])
function usable(a: any) {
  const scope = state.app?.settings.providers?.[a.provider as 'workbuddy']
  return (
    a.enabled &&
    !a.blocked &&
    !a.authFailed &&
    !(a.cooldownUntil > state.now) &&
    !(a.expiresAt && a.expiresAt < state.now) &&
    scope?.enabled &&
    scope.regions.includes(a.region)
  )
}
const byProvider = computed(() =>
  PROVIDERS.map((p) => {
    const list = runtime.value.filter((a) => a.provider === p.key)
    const credits = list
      .filter((a) => a.credit?.unit === 'credits' && a.credit?.remaining != null)
      .reduce((s, a) => s + a.credit.remaining, 0)
    return {
      ...p,
      total: list.length,
      usable: list.filter(usable).length,
      cn: list.filter((a) => a.region === 'cn').length,
      intl: list.filter((a) => a.region === 'intl').length,
      credits,
      enabled: state.app?.settings.providers?.[p.key]?.enabled ?? true,
    }
  })
)
const routes = computed(() => state.catalog?.routes ?? [])
const shared = computed(() =>
  routes.value.filter(
    (r) => r.available && new Set(r.targets.map((x) => x.provider + x.region)).size > 1
  )
)
const proxyLogs = computed(() => state.logs.filter((l) => l.protocol !== 'internal'))
const tokens = computed(() =>
  proxyLogs.value.reduce(
    (s, l) => s + (requestTokenUsage(l.usage ?? l.details?.usage).total ?? 0),
    0
  )
)
const success = computed(() => proxyLogs.value.filter((l) => l.outcome === 'completed').length)
const strategyName = computed(() => {
  const s = state.app?.settings.strategy
  return s === 'creditFirst'
    ? t('积分优先', 'Credits first')
    : s === 'expiryFirst'
      ? t('临期优先', 'Expiring first')
      : s === 'ordered'
        ? t('顺序优先', 'Ordered')
        : s === 'random'
          ? t('随机', 'Random')
          : t('轮询', 'Round robin')
})
</script>

<template>
  <section class="hero">
    <div>
      <div class="eyebrow">
        <span
          class="dot"
          :class="{ online: running }"
          style="display: inline-block; margin-right: 6px"
        ></span
        >{{
          running
            ? t('本地网关正在运行', 'LOCAL GATEWAY RUNNING')
            : t('本地网关待启动', 'LOCAL GATEWAY STOPPED')
        }}
      </div>
      <h2>{{ t('三个端，一个入口。', 'Three providers. One endpoint.') }}</h2>
      <div class="endpoint">
        <code class="mono">{{ base }}</code
        ><a-button size="mini" type="text" @click="copy(base)"><IconCopy /></a-button>
      </div>
      <p>
        {{
          t(
            '同一模型在 WorkBuddy、Qoder、Trae 的国内与国际账号之间统一调度；对外只暴露你设置的模型别名。',
            'The same model is scheduled across WorkBuddy, Qoder and Trae accounts in both regions; clients only see your aliases.'
          )
        }}
      </p>
    </div>
    <div class="hero-marks">
      <ProviderMark provider="workbuddy" /><ProviderMark provider="qoder" /><ProviderMark
        provider="trae"
      />
    </div>
  </section>

  <div class="metric-row">
    <div v-for="p in byProvider" :key="p.key" class="metric">
      <span
        ><ProviderMark :provider="p.key" style="margin-right: 6px" />{{ p.label
        }}<span v-if="!p.enabled" class="state-pill off" style="margin-left: 6px">{{
          t('未参与', 'Off')
        }}</span></span
      >
      <strong
        >{{ p.usable
        }}<small>/ {{ p.total }} {{ t('个可用账号', 'usable accounts') }}</small></strong
      >
      <p>
        {{ t('国内', 'CN') }} {{ p.cn }} · {{ t('国际', 'Intl') }} {{ p.intl
        }}<template v-if="p.credits">
          · {{ fmtNumber(p.credits) }} {{ t('积分', 'credits') }}</template
        >
      </p>
    </div>
    <div class="metric">
      <span>{{ t('对外模型', 'Public models') }}</span>
      <strong
        >{{ routes.filter((r) => r.available).length }}<small>{{ t('个', '') }}</small></strong
      >
      <p>
        {{ shared.length }}
        {{ t('个模型跨端/跨地区共用账号池', 'shared across providers/regions') }}
      </p>
    </div>
  </div>

  <div class="split-2">
    <section class="box">
      <div class="box-head">
        <div>
          <h2>{{ t('最近请求', 'Recent requests') }}</h2>
          <p>{{ t('仅保存运行元数据', 'Runtime metadata only') }}</p>
        </div>
        <a-button size="small" type="text" @click="page = 'requests'"
          >{{ t('查看全部', 'View all') }} <IconArrowRight
        /></a-button>
      </div>
      <div
        v-for="l in [
          ...state.active.map((x) => ({ ...x, outcome: 'running' })),
          ...proxyLogs,
        ].slice(0, 8)"
        :key="l.id"
        class="kv"
      >
        <span
          >{{ fmtShort(l.time) }} ·
          <b style="font-weight: 600">{{ l.model || l.requestedModel }}</b></span
        >
        <b>
          <span v-if="l.provider" class="ptag"
            ><ProviderMark :provider="l.provider" />{{ providerLabel(l.provider) }} ·
            {{ regionLabel(l.region) }}</span
          >
          <span
            class="state-pill"
            :class="{
              bad: l.outcome !== 'completed' && l.outcome !== 'running',
              warn: l.outcome === 'running',
            }"
            style="margin-left: 6px"
            >{{ outcomeName(l.outcome) }}</span
          >
          <small class="muted" style="margin-left: 6px">{{
            l.outcome === 'running' ? '…' : duration(l.durationMs)
          }}</small>
        </b>
      </div>
      <a-empty
        v-if="!proxyLogs.length && !state.active.length"
        :description="
          t(
            '启动服务并连接客户端后，每次调用都会出现在这里。',
            'Start the gateway and connect a client to see requests here.'
          )
        "
      />
    </section>
    <div>
      <section class="box">
        <div class="box-head">
          <h2>{{ t('运行概况', 'Runtime') }}</h2>
        </div>
        <div class="kv">
          <span>{{ t('服务状态', 'Service') }}</span
          ><b>{{
            running
              ? t('正在监听', 'Listening') +
                ' 127.0.0.1:' +
                state.app?.port +
                (state.app?.lan && state.app.lanAddresses?.[0] ? ` · ${state.app.lanAddresses[0]}:${state.app.port}` : '')
              : t('未启动', 'Stopped')
          }}</b>
        </div>
        <div class="kv">
          <span>{{ t('调度策略', 'Scheduling') }}</span
          ><b>{{ strategyName }}</b>
        </div>
        <div class="kv">
          <span>{{ t('进行中请求', 'Active requests') }}</span
          ><b>{{ state.active.length }}</b>
        </div>
        <div class="kv">
          <span>{{ t('最近成功率', 'Recent success rate') }}</span
          ><b>{{
            proxyLogs.length ? Math.round((success / proxyLogs.length) * 100) + '%' : '—'
          }}</b>
        </div>
        <div class="kv">
          <span>{{ t('已记录 Tokens', 'Recorded tokens') }}</span
          ><b>{{ tokenCount(tokens) }}</b>
        </div>
      </section>
      <section class="box client-list">
        <div class="box-head">
          <h2>{{ t('快速接入', 'Quick connection') }}</h2>
        </div>
        <button @click="page = 'access'">
          <span class="cmark">✳</span
          ><span><b>Claude Code</b><small>Anthropic Messages</small></span>
        </button>
        <button @click="page = 'access'">
          <span class="cmark">⌘</span><span><b>Codex</b><small>OpenAI Responses</small></span>
        </button>
        <button @click="page = 'access'">
          <span class="cmark">⌁</span
          ><span><b>OpenAI Compatible</b><small>Chat Completions</small></span>
        </button>
      </section>
    </div>
  </div>
</template>
