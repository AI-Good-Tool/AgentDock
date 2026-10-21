<script setup lang="ts">
import ProviderMark from '../components/ProviderMark.vue'
import { computed, ref, onMounted } from 'vue'
import { Message, Modal } from '@arco-design/web-vue'
import { IconCalendar, IconGift, IconDelete } from '@arco-design/web-vue/es/icon'
import PageHead from '../components/PageHead.vue'
import CheckinPage from '../components/workbuddy/CheckinPage.vue'
import { call } from '../api'
import { t, fmtDateTime, fmtNumber } from '../i18n'
import { state, act, busy, refreshPool } from '../store'

const tab = ref<'workbuddy' | 'qoder' | 'trae'>('workbuddy')
const progress = ref({ completed: 0, total: 0, failed: 0 })
onMounted(refreshPool)
const wb = computed(() => state.pool?.workbuddy ?? { accounts: [] })
const pendingIds = computed(() => [...state.pending].map((k) => k.replace(/^wb:/, '')))

// WorkBuddy
function wbToggleAuto(enabled: boolean) {
  const apply = () =>
    act(
      'auto-checkin',
      () => call('wb_set_auto_checkin', { enabled }),
      t('自动签到设置已保存', 'Auto check-in saved')
    )
  if (!enabled) return apply()
  Modal.confirm({
    title: t('开启每日自动签到', 'Enable daily auto check-in'),
    content: t(
      '应用运行期间，在设置的上海时间窗口内为已开启签到的国内版账号分散执行状态核验与签到；失败会限频重试。与反代开关独立，可能触发官方限流。',
      'While the app runs, opted-in China accounts are checked in within the configured Shanghai-time window, spread out, with throttled retries. Independent of the proxy switch; may hit official rate limits.'
    ),
    onOk: apply,
  })
}
async function wbBatch(submit = true) {
  await act('checkin-all', async () => {
    const accounts = wb.value.accounts.filter(
      (a: any) => a.variant === 'cn' && (!submit || a.checkinEnabled)
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
const claimLog = computed(() => [...(q.value.claimLog ?? [])].reverse())
const phase: Record<string, [string, string]> = {
  claimed: ['领取成功', 'Claimed'],
  nothing: ['无可领', 'Nothing to claim'],
  dry_run: ['干跑', 'Dry run'],
  login_required: ['需重新登录', 'Login required'],
  error: ['失败', 'Failed'],
}
function clearQoderLog() {
  Modal.confirm({
    title: t('清除领取记录', 'Clear claim log'),
    content: t('清除后不可恢复。', 'This cannot be undone.'),
    onOk: () => act('qclear', () => call('qoder_clear_claim_log')),
  })
}

// Trae
const traeCn = computed(() =>
  (state.pool?.trae?.accounts ?? []).filter((a: any) => a.variant === 'cn')
)
async function traeAll() {
  await act('trae-checkin-all', async () => {
    let failed = 0
    for (const a of traeCn.value) await call('trae_checkin', { id: a.id }).catch(() => failed++)
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
        'WorkBuddy 每日签到、Qoder 每日积分领取与 Trae 国内版签到，各自保留原有的规则与记录。',
        'WorkBuddy check-in, Qoder daily credits and Trae CN check-in, each with its original rules and history.'
      )
    "
  />
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
    :snapshot="wb as any"
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
          ><small>{{
            t('应用运行时每 10 分钟检查一次可领活动', 'Checks every 10 minutes while running')
          }}</small>
        </div>
        <div class="control">
          <a-switch
            :model-value="q.settings?.autoClaim !== false"
            @change="(v) => act('qauto', () => call('qoder_set_auto_claim', { enabled: !!v }))"
          />
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
            '不打开应用也能领取：在「偏好设置」安装系统定时任务（每天 10:05 / 22:05）。',
            'To claim without the app, install the scheduled task in Preferences (daily 10:05 / 22:05).'
          )
        }}
      </p>
    </section>
    <section class="box">
      <div class="box-head">
        <h2>{{ t('领取记录', 'Claim log') }}</h2>
        <a-button size="small" @click="clearQoderLog"
          ><template #icon><IconDelete /></template>{{ t('清除记录', 'Clear') }}</a-button
        >
      </div>
      <div v-for="(entry, i) in claimLog" :key="i" style="margin-bottom: 10px">
        <b style="font-size: 13px">{{ entry.time }}</b>
        <div v-for="(r, j) in entry.results" :key="j" class="kv">
          <span
            >{{ r.name }} ·
            {{ r.edition === 'cn' ? t('国内版', 'China') : t('国际版', 'International') }}</span
          >
          <b
            ><span
              class="state-pill"
              :class="{
                bad: r.phase === 'error' || r.phase === 'login_required',
                off: r.phase === 'nothing',
              }"
              >{{ phase[r.phase] ? t(...phase[r.phase]) : r.phase }}</span
            >
            {{ r.credits ? '+' + fmtNumber(r.credits) : '' }}
            <small class="muted">{{ r.msg }}</small></b
          >
        </div>
      </div>
      <a-empty v-if="!claimLog.length" :description="t('暂无记录', 'No records yet')" />
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
                '先查询服务端状态，已领取时不会重复请求；按北京时间判断签到日。开启自动签到的账号在启动时补签，此后每 10 分钟检查一次，需要保持应用运行。国际版没有相同签到入口。',
                'Status is queried first, so claimed days are never re-submitted; days follow Beijing time. Opted-in accounts catch up at launch and every 10 minutes while the app runs. International has no check-in.'
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
    <section class="box">
      <div class="box-head">
        <h2>{{ t('签到记录', 'Check-in history') }}</h2>
        <span class="muted">{{ t('最近 300 条', 'Latest 300') }}</span>
      </div>
      <a-table
        :data="(state.pool?.trae?.checkinHistory ?? []).slice(0, 100)"
        :pagination="{ pageSize: 10 }"
        :bordered="false"
        class="table-tight"
      >
        <template #columns>
          <a-table-column :title="t('时间', 'Time')"
            ><template #cell="{ record }">{{ fmtDateTime(record.time) }}</template></a-table-column
          >
          <a-table-column :title="t('账号', 'Account')" data-index="accountName" />
          <a-table-column :title="t('触发方式', 'Trigger')"
            ><template #cell="{ record }">{{
              record.source === 'auto' ? t('自动', 'Auto') : t('手动', 'Manual')
            }}</template></a-table-column
          >
          <a-table-column :title="t('结果', 'Result')"
            ><template #cell="{ record }">{{
              record.status === 'already'
                ? t('已领取', 'Already claimed')
                : record.status === 'success'
                  ? t('签到成功', 'Checked in')
                  : record.error
            }}</template></a-table-column
          >
        </template>
      </a-table>
    </section>
  </template>
</template>
