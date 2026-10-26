<script setup lang="ts">
import ProviderMark from '../components/ProviderMark.vue'
import { computed, ref, onMounted } from 'vue'
import { Message, Modal } from '@arco-design/web-vue'
import { IconCalendar, IconGift, IconPlayArrow, IconClockCircle } from '@arco-design/web-vue/es/icon'
import PageHead from '../components/PageHead.vue'
import CheckinPage from '../components/workbuddy/CheckinPage.vue'
import CheckinHistory from '../components/CheckinHistory.vue'
import CheckinScheduleEditor from '../components/CheckinScheduleEditor.vue'
import { defaultCheckinSchedule, describeCheckinSchedule } from '../checkin-schedule'
import { call, isWindows, type CheckinSettings, type ProviderKey } from '../api'
import { t, fmtDateTime } from '../i18n'
import { state, act, busy, refreshPool, refreshApp, isDisabled } from '../store'

const tab = ref<'workbuddy' | 'qoder' | 'trae'>('workbuddy')
const progress = ref({ completed: 0, total: 0, failed: 0 })
const launchd = ref<{ installed: boolean; legacy?: boolean } | null>(null)
onMounted(async () => {
  refreshPool()
  launchd.value = await call('launchd_status').catch(() => null)
})

// Unified schedule (all three platforms)
const plan = computed<CheckinSettings>(
  () => state.app?.settings.checkin ?? { workbuddy: true, qoder: true, trae: true, onStartup: true, scheduled: true, intervalMinutes: 60 }
)
const status = computed(() => state.app?.checkin ?? {})
// Partial patches (merged server-side) with a key per field, so quick toggles never drop or revert each other.
function setPlan(patch: Partial<CheckinSettings>, success?: string) {
  return act(`checkin:${Object.keys(patch).join(',')}`, () => call('save_settings', { patch: { checkin: patch } }), success ?? t('签到设置已保存', 'Check-in settings saved'), [refreshApp])
}
const schedule = computed(() => plan.value.schedule ?? (plan.value.intervalMinutes !== 60
  ? { ...defaultCheckinSchedule(), kind: 'minutes' as const, every: plan.value.intervalMinutes }
  : defaultCheckinSchedule()))
const scheduleHint = computed(() => {
  const parts: string[] = []
  if (plan.value.onStartup) parts.push(t('启动后 10 秒', '10 s after launch'))
  if (plan.value.scheduled) parts.push(describeCheckinSchedule(schedule.value))
  return parts.length ? parts.join(t('，之后', ', then ')) + t('执行', '') : t('仅手动执行', 'Manual only')
})
const platformTiles = computed(() => [
  { key: 'workbuddy' as ProviderKey, name: 'WorkBuddy', desc: t('国内版中已开启签到的账号', 'China accounts with check-in on') },
  { key: 'qoder' as ProviderKey, name: 'Qoder', desc: t('全部账号的每日积分活动', 'Daily credit campaigns of all accounts') },
  { key: 'trae' as ProviderKey, name: 'Trae', desc: t('国内版账号（定时只签开启「自动」的）', 'China accounts (schedule: only those with Auto on)') },
])
const triggerName: Record<string, [string, string]> = {
  startup: ['启动后自动', 'At launch'],
  scheduled: ['定时', 'Scheduled'],
  manual: ['手动', 'Manual'],
  launchd: ['系统定时任务', 'launchd'],
}
function resultText(key: ProviderKey) {
  const rows = status.value.results?.[key]
  if (rows === 'disabled') return t('已关闭', 'Off')
  if (!Array.isArray(rows)) return '—'
  if (!rows.length) return t('无需处理', 'Nothing due')
  const failed = rows.filter((r: any) => r.ok === false || r.phase === 'error' || r.phase === 'login_required').length
  return failed ? t(`${rows.length - failed} 成功 · ${failed} 失败`, `${rows.length - failed} ok · ${failed} failed`) : t(`${rows.length} 个已处理`, `${rows.length} done`)
}
async function runNow() {
  await act('checkin-run', () => call('checkin_run_now'), t('签到已执行', 'Check-in pass finished'), [refreshApp, refreshPool])
}
function toggleLaunchd(forceInstall = false) {
  const install = forceInstall || !launchd.value?.installed
  return act(
    'launchd',
    async () => {
      const r = await call('launchd_install', { install })
      launchd.value = await call('launchd_status')
      return r
    },
    install
      ? t('已安装：无需打开应用，按上面的签到计划执行', 'Installed: follows the check-in schedule without the app')
      : t('已移除系统定时任务', 'Scheduled task removed'),
    []
  )
}
const wb = computed(() => state.pool?.workbuddy ?? { accounts: [] })
const wbView = computed(() => ({
  ...(wb.value as any),
  autoCheckin: plan.value.workbuddy,
  accounts: wb.value.accounts.map((a: any) => ({ ...a, disabled: isDisabled('workbuddy', a.id) })),
}))
const pendingIds = computed(() => [...state.pending].map((k) => k.replace(/^wb:/, '')))

// WorkBuddy
function wbToggleAuto(enabled: boolean) {
  const apply = () => setPlan({ workbuddy: enabled }, t('自动签到设置已保存', 'Auto check-in saved'))
  if (!enabled) return apply()
  Modal.confirm({
    title: t('开启每日自动签到', 'Enable daily auto check-in'),
    content: t(
      '应用运行期间按签到计划（启动后 / 定时）为已开启签到的国内版账号核验状态并签到，已签到的账号不会重复提交。与反代开关独立，可能触发官方限流。',
      'While the app runs, opted-in China accounts are checked and checked in on the schedule (at launch / interval); accounts already checked in are skipped. Independent of the proxy switch; may hit official rate limits.'
    ),
    onOk: apply,
  })
}
async function wbBatch(submit = true) {
  await act('checkin-all', async () => {
    const accounts = wb.value.accounts.filter(
      (a: any) => a.variant === 'cn' && (!submit || a.checkinEnabled) && !isDisabled('workbuddy', a.id)
    )
    progress.value = { completed: 0, total: accounts.length, failed: 0 }
    for (const a of accounts) {
      try {
        await call('wb_checkin', { id: a.id, submit })
      } catch {
        progress.value.failed++
      } finally {
        progress.value.completed++
        await refreshPool()
      }
    }
    const { completed, failed } = progress.value
    const msg = t(
      `完成 ${completed - failed} 个，失败 ${failed} 个`,
      `${completed - failed} done, ${failed} failed`
    )
    failed ? Message.warning(msg) : Message.success(msg)
  })
}

// Qoder
const q = computed(() => state.pool?.qoder ?? {})

// Trae
const traeCn = computed(() =>
  (state.pool?.trae?.accounts ?? []).filter((a: any) => a.variant === 'cn')
)
async function traeAll() {
  await act('trae-checkin-all', async () => {
    let failed = 0
    for (const a of traeCn.value.filter((a: any) => !isDisabled('trae', a.id)))
      await call('trae_checkin', { id: a.id }).catch(() => failed++)
    failed
      ? Message.warning(t(`${failed} 个账号签到未完成`, `${failed} accounts failed`))
      : Message.success(t('签到状态已确认', 'Check-ins confirmed'))
  })
}
</script>

<template>
  <PageHead
    eyebrow="DAILY REWARDS"
    :title="t('签到与领取', 'Daily Rewards')"
    :desc="
      t(
        'WorkBuddy 每日签到、Qoder 每日积分领取与 Trae 国内版签到：统一的自动签到计划，各平台独立开关与记录。',
        'WorkBuddy check-in, Qoder daily credits and Trae CN check-in: one schedule, a switch and history per platform.'
      )
    "
  />
  <section class="box checkin-plan">
    <div class="box-head">
      <div>
        <h2>{{ t('自动签到计划', 'Auto check-in schedule') }}</h2>
        <p>{{ t('三个平台共用一个计划，分别开关。「立即执行一次」处理已开启平台的全部账号（WorkBuddy 为参与批量签到的账号）；启动后与定时执行时，Trae 只签开启「自动」的账号，Qoder 只在有可领积分时领取。已签到 / 已领取的不会重复提交；号池中已禁用的账号始终不参与。', 'One schedule for all three platforms, each with its own switch. “Run now” covers every account of the enabled platforms (WorkBuddy: accounts in batch check-in); at launch and on schedule, Trae checks in only accounts with Auto on and Qoder claims only when credits are available. Accounts already done are skipped; accounts disabled in the pool never take part.') }}</p>
      </div>
      <a-button type="primary" :loading="busy('checkin-run') || !!status.running" @click="runNow"><template #icon><IconPlayArrow /></template>{{ t('立即执行一次', 'Run now') }}</a-button>
    </div>
    <div class="plan-tiles">
      <div v-for="p in platformTiles" :key="p.key" class="plan-tile" :class="{ off: !plan[p.key] }">
        <ProviderMark :provider="p.key" />
        <div>
          <b>{{ p.name }}</b>
          <small>{{ p.desc }}</small>
          <small class="plan-result">{{ t('上次：', 'Last: ') }}{{ resultText(p.key) }}</small>
        </div>
        <a-switch :model-value="plan[p.key]" :loading="busy(`checkin:${p.key}`)" @change="(v) => setPlan({ [p.key]: !!v })" />
      </div>
    </div>
    <div class="field-row">
      <div><b>{{ t('启动后自动签到', 'Check in at launch') }}</b><small>{{ t('AgentDock 启动 10 秒后执行一次', 'Runs once 10 seconds after AgentDock starts') }}</small></div>
      <div class="control end"><a-switch :model-value="plan.onStartup" @change="(v) => setPlan({ onStartup: !!v })" /></div>
    </div>
    <div class="field-row">
      <div><b>{{ t('定时签到', 'Scheduled check-in') }}</b><small>{{ t('支持间隔、每天、指定日期和星期，默认每天 07:00', 'Intervals, daily, specific dates or weekdays; defaults to 07:00 daily') }}</small></div>
      <div class="control end">
        <a-switch :model-value="plan.scheduled" @change="(v) => setPlan({ scheduled: !!v })" />
      </div>
    </div>
    <CheckinScheduleEditor :model-value="schedule" :disabled="!plan.scheduled" :saving="busy('checkin:schedule')" @save="(schedule) => setPlan({ schedule })" />
    <div class="field-row">
      <div><b>{{ t('执行状态', 'Status') }}</b><small>{{ scheduleHint }}</small></div>
      <div class="control end plan-status">
        <span><IconClockCircle /> {{ t('上次', 'Last') }}：{{ status.lastRun ? fmtDateTime(status.lastRun) : t('尚未执行', 'Not yet') }}<template v-if="status.trigger && triggerName[status.trigger]">（{{ t(...triggerName[status.trigger]) }}）</template></span>
        <span>{{ t('下次', 'Next') }}：{{ status.running ? t('执行中…', 'Running…') : status.nextRun ? fmtDateTime(status.nextRun) : t('未计划', 'Not scheduled') }}</span>
      </div>
    </div>
    <div class="field-row" style="border-bottom: none">
      <div><b>{{ isWindows ? t('系统定时任务（任务计划程序）', 'Scheduled task (Task Scheduler)') : t('系统定时任务', 'Scheduled task (launchd)') }}</b><small>{{ t('不开应用也能签到：每分钟检查上面的计划，按偏好设置时区执行；修改计划无需重新安装', 'Checks the shared schedule every minute without the app; uses the preferred time zone and picks up schedule changes automatically') }}</small></div>
      <div class="control end">
        <span class="state-pill" :class="{ off: !launchd?.installed || launchd?.legacy }">{{ launchd?.legacy ? t('需更新', 'Update required') : launchd?.installed ? t('已安装', 'Installed') : t('未安装', 'Not installed') }}</span>
        <a-button v-if="launchd?.legacy" size="small" type="primary" :loading="busy('launchd')" @click="toggleLaunchd(true)">{{ t('更新任务', 'Update task') }}</a-button>
        <a-button size="small" :loading="busy('launchd')" @click="toggleLaunchd()">{{ launchd?.installed ? t('移除', 'Remove') : t('安装', 'Install') }}</a-button>
      </div>
    </div>
  </section>

  <a-radio-group v-model="tab" type="button" style="margin-bottom: 18px">
    <a-radio value="workbuddy"
      ><ProviderMark provider="workbuddy" style="margin-right: 6px" />WorkBuddy</a-radio
    >
    <a-radio value="qoder"
      ><ProviderMark provider="qoder" style="margin-right: 6px" />Qoder</a-radio
    >
    <a-radio value="trae"><ProviderMark provider="trae" style="margin-right: 6px" />Trae</a-radio>
  </a-radio-group>

  <CheckinPage
    v-if="tab === 'workbuddy'"
    :snapshot="wbView"
    :auto-hint="scheduleHint"
    :pending-ids="pendingIds"
    :progress="progress"
    :now="state.now"
    @toggle-auto="wbToggleAuto"
    @save-config="
      (config) =>
        act(
          'checkin-settings',
          () => call('wb_set_checkin_config', { config }),
          t('签到时间段已保存', 'Check-in window saved')
        )
    "
    @toggle-account="
      (a, enabled) => act(`wb:${a.id}`, () => call('wb_set_account_checkin', { id: a.id, enabled }))
    "
    @checkin="
      (a, submit) =>
        act(
          `wb:${a.id}`,
          () => call('wb_checkin', { id: a.id, submit }),
          submit
            ? t('签到状态已更新', 'Check-in updated')
            : t('已查询官方签到状态', 'Status queried')
        )
    "
    @batch="wbBatch"
  />

  <template v-if="tab === 'qoder'">
    <section class="box">
      <div class="box-head">
        <div>
          <h2>{{ t('每日积分自动领取', 'Daily credit auto-claim') }}</h2>
          <p>
            {{
              t(
                '「每天 100 Credits」等活动自动领取；接口幂等，不会重复发放。',
                'Claims daily campaigns such as “100 credits per day”; idempotent.'
              )
            }}
          </p>
        </div>
        <a-button
          type="primary"
          :loading="busy('qclaim-all')"
          @click="act('qclaim-all', () => call('qoder_claim', {}))"
          ><template #icon><IconGift /></template>{{ t('立即全部领取', 'Claim all now') }}</a-button
        >
      </div>
      <div class="field-row">
        <div>
          <b>{{ t('应用内自动领取', 'Auto-claim in app') }}</b
          ><small>{{ scheduleHint }}</small>
        </div>
        <div class="control">
          <a-switch :model-value="plan.qoder" @change="(v) => setPlan({ qoder: !!v })" />
        </div>
      </div>
      <div class="field-row">
        <div>
          <b>{{ t('上次自动执行', 'Last automatic run') }}</b>
        </div>
        <div class="control">{{ q.scheduler?.lastRun || t('尚未执行', 'Not yet') }}</div>
      </div>
      <p class="hint">
        {{
          t(
            '不打开应用也能领取：在上方「自动签到计划」安装系统定时任务，按同一计划执行。',
            'To claim without the app, install the system task above; it follows the same schedule.'
          )
        }}
      </p>
    </section>
  </template>

  <template v-if="tab === 'trae'">
    <section class="box">
      <div class="box-head">
        <div>
          <h2>{{ t('Trae CN 每日签到', 'Trae CN daily check-in') }}</h2>
          <p>
            {{
              t(
                '先查询服务端状态，已领取时不会重复请求；按北京时间判断签到日。开启「自动」的账号按上方签到计划执行，需要保持应用运行；「立即执行一次」会签到全部国内账号（已禁用的除外）。国际版没有相同签到入口。',
                'Status is queried first, so claimed days are never re-submitted; days follow Beijing time. Accounts with “Auto” on follow the schedule above while the app runs; “Run now” checks in every CN account except disabled ones. International has no check-in.'
              )
            }}
          </p>
        </div>
        <a-button type="primary" :loading="busy('trae-checkin-all')" @click="traeAll"
          ><template #icon><IconCalendar /></template
          >{{ t('签到全部国内账号', 'Check in all CN accounts') }}</a-button
        >
      </div>
      <div v-for="a in traeCn" :key="a.id" class="field-row">
        <div>
          <b>{{ a.note || a.username }}</b>
          <small>{{
            a.checkin
              ? t('最后查询于 ', 'Last checked ') + fmtDateTime(a.checkin.time)
              : t('点击签到将先检查服务端状态', 'Checking first queries the server')
          }}</small>
        </div>
        <div class="control">
          <span v-if="isDisabled('trae', a.id)" class="state-pill off">{{ t('已禁用', 'Disabled') }}</span>
          <span class="state-pill" :class="{ warn: !a.checkedToday }">{{
            a.checkedToday
              ? t('今日已签到', 'Checked in today')
              : a.checkin
                ? t('待签到', 'Pending')
                : t('等待查询', 'Not queried')
          }}</span>
          <span class="muted">{{ a.checkin?.credits ?? '—' }} credits</span>
          <a-button
            size="small"
            :loading="busy(`trae:${a.id}:trae_checkin`)"
            @click="
              act(
                `trae:${a.id}:trae_checkin`,
                () => call('trae_checkin', { id: a.id }),
                t('签到状态已确认', 'Check-in confirmed')
              )
            "
            >{{ t('检查并签到', 'Check in') }}</a-button
          >
          <span class="muted">{{ t('自动', 'Auto') }}</span>
          <a-switch
            size="small"
            :model-value="a.autoCheckin"
            :disabled="isDisabled('trae', a.id)"
            @change="
              (v) =>
                act(`trae:${a.id}:auto`, () =>
                  call('trae_update_account', { id: a.id, patch: { autoCheckin: !!v } })
                )
            "
          />
        </div>
      </div>
      <a-empty
        v-if="!traeCn.length"
        :description="t('添加一个 Trae CN 账号后可签到', 'Add a Trae CN account to check in')"
      />
    </section>
  </template>

  <CheckinHistory />
</template>
