<script setup lang="ts">
import ProviderMark from '../components/ProviderMark.vue'
import { computed, ref, onMounted, h } from 'vue'
import { Modal, Message } from '@arco-design/web-vue'
import { IconPlus, IconRefresh, IconThunderbolt, IconLaunch, IconEye, IconEyeInvisible } from '@arco-design/web-vue/es/icon'
import PageHead from '../components/PageHead.vue'
import AccountCard from '../components/workbuddy/AccountCard.vue'
import AddAccountDialog from '../components/workbuddy/AddAccountDialog.vue'
import WbDrawers from '../components/WbDrawers.vue'
import QoderCard from '../components/QoderCard.vue'
import QoderSwitchDialog from '../components/QoderSwitchDialog.vue'
import QoderInfoDialog from '../components/QoderInfoDialog.vue'
import TraeCard from '../components/TraeCard.vue'
import TraeDialogs from '../components/TraeDialogs.vue'
import LoginDialog from '../components/LoginDialog.vue'
import AccountJsonDialog from '../components/AccountJsonDialog.vue'
import AccountBatchExportDialog from '../components/AccountBatchExportDialog.vue'
import { call } from '../api'
import { t } from '../i18n'
import { state, act, busy, refreshPool, runtimeOf, revealPhones, isDisabled } from '../store'
import { accountName } from '../lib/account-display'

const provider = ref<'all' | 'workbuddy' | 'qoder' | 'trae'>('all')
const region = ref<'all' | 'cn' | 'intl'>('cn')
const query = ref('')
const compact = ref(false)
const creditLoading = ref<string[]>([])
const dialog = ref<null | { kind: string; [k: string]: any }>(null)
onMounted(() => {
  refreshPool()
  call('wb_refresh_statuses')
    .then(refreshPool)
    .catch(() => {})
})

const match = (...values: unknown[]) =>
  values.some((v) =>
    String(v ?? '')
      .toLowerCase()
      .includes(query.value.trim().toLowerCase())
  )
const inRegion = (r: string) => region.value === 'all' || r === region.value
const show = (p: string) => provider.value === 'all' || provider.value === p

const wbAccounts = computed(() =>
  (state.pool?.workbuddy?.accounts ?? [])
    .filter(
      (a: any) =>
        inRegion(a.variant === 'ai' ? 'intl' : 'cn') &&
        match(accountName(a), a.presentation?.uid, a.id, a.note, a.presentation?.phoneNumber)
    )
    .map((a: any) => {
      const rt = runtimeOf('workbuddy', a.id)
      return {
        ...a,
        inFlight: rt?.inFlight ?? 0,
        concurrency: rt?.concurrency ?? 4,
        authFailed: !!(rt?.authFailed || a.authFailed),
        enabled: rt?.enabled ?? a.enabled,
        disabled: !!rt?.disabled,
        cooldownUntil: rt?.cooldownUntil,
      }
    })
)
const qoderAccounts = computed(() =>
  (state.pool?.qoder?.accounts ?? []).filter(
    (a: any) => inRegion(a.edition) && match(a.name, a.note, a.email, a.phone, a.uid)
  )
)
const traeAccounts = computed(() =>
  (state.pool?.trae?.accounts ?? []).filter(
    (a: any) => inRegion(a.variant) && match(a.username, a.note, a.email, a.userId, a.mobile)
  )
)
const total = computed(
  () =>
    (show('workbuddy') ? wbAccounts.value.length : 0) +
    (show('qoder') ? qoderAccounts.value.length : 0) +
    (show('trae') ? traeAccounts.value.length : 0)
)
const exportAccounts = computed(() => [
  ...(show('workbuddy') ? wbAccounts.value.map((a: any) => ({ provider: 'workbuddy', id: a.id, name: accountName(a), region: a.variant === 'ai' ? 'intl' : 'cn' })) : []),
  ...(show('qoder') ? qoderAccounts.value.map((a: any) => ({ provider: 'qoder', id: a.id, name: a.note || a.name || a.email || a.uid, region: a.edition })) : []),
  ...(show('trae') ? traeAccounts.value.map((a: any) => ({ provider: 'trae', id: a.id, name: a.note || a.username || a.userId, region: a.variant })) : []),
])

// ── WorkBuddy ──
function wbEnable(a: any, enabled: boolean) {
  return act(`wb:${a.id}`, () =>
    call('set_account_enabled', { providerKey: 'workbuddy', id: a.id, enabled })
  )
}
function wbCheckinEnable(a: any, enabled: boolean) {
  return act(
    `wb:${a.id}`,
    () => call('wb_set_account_checkin', { id: a.id, enabled }),
    enabled ? t('此账号签到已开启', 'Check-in enabled') : t('此账号签到已关闭', 'Check-in disabled')
  )
}
function wbRenew(a: any) {
  return act(`wb:${a.id}`, () => call('wb_renew_token', { id: a.id }))
}
function wbCheckin(a: any) {
  return act(
    `wb:${a.id}`,
    () => call('wb_checkin', { id: a.id, submit: false }),
    t('已查询官方签到状态', 'Check-in status updated')
  )
}
async function wbCredits(ids: string[]) {
  creditLoading.value = ids
  try {
    await act('wb-credits', async () => {
      const r = await call('wb_refresh_credits', { ids })
      const good = r.results.filter((x: any) => x.ok).length
      if (good === r.results.length)
        Message.success(t(`已更新 ${good} 个账号积分`, `Updated ${good} accounts`))
      else
        Message.warning(
          t(
            `${good} 个成功，${r.results.length - good} 个失败；详情见账号卡片`,
            `${good} updated, ${r.results.length - good} failed; see cards`
          )
        )
    })
  } finally {
    creditLoading.value = []
  }
}
function wbSwitch(a: any, target: string) {
  const names: Record<string, string> = {
    workbuddy: 'WorkBuddy',
    ide: 'CodeBuddy IDE',
    vscode: 'VS Code',
    cli: 'CodeBuddy CLI',
  }
  Modal.confirm({
    title: t(`切换 ${names[target]} 账号`, `Switch ${names[target]} account`),
    content: t(
      `将 ${names[target]} 切换为「${accountName(a)}」。此操作会备份并改写目标客户端登录态，关闭正在运行的目标客户端；CLI 会话会中断，其余目标会重新打开。请先保存工作。`,
      `Switch ${names[target]} to “${accountName(a)}”. The target client's login is backed up and rewritten and the running client is closed (CLI sessions end; others reopen). Save your work first.`
    ),
    okText: t('确认切换', 'Switch'),
    cancelText: t('取消', 'Cancel'),
    onOk: () =>
      act(
        'wb-switch',
        () => call('wb_switch_client', { id: a.id, target, confirmed: true }),
        t('客户端账号已切换', 'Client account switched')
      ),
  })
}
function wbImport(source: string) {
  return act(
    'wb-import',
    async () => {
      const result = await call<any>('wb_import_accounts', { source })
      await refreshPool()
      Message.success(result?.updated
        ? t('当前登录凭据已更新，账号状态已刷新', 'Current login credentials and account status updated')
        : t(`已导入 ${result?.count ?? result ?? 0} 个账号`, `Imported ${result?.count ?? result ?? 0} accounts`))
      return result
    }
  )
}

// ── Qoder ──
function qoderClaim(a: any) {
  return act(`qclaim:${a.id}`, () => call('qoder_claim', { id: a.id }))
}
function qoderCredits(ids: string[]) {
  return act(`qcredit:${ids.length === 1 ? ids[0] : 'all'}`, async () => {
    let failed = 0
    for (const id of ids) {
      const r = await call('qoder_credits', { id }).catch(() => ({ ok: false }))
      if (!r.ok) failed++
    }
    if (failed) Message.warning(t(`${failed} 个账号查询失败`, `${failed} accounts failed`))
    else Message.success(t('积分已更新', 'Credits updated'))
  })
}
function qoderCapture(edition: string) {
  return act('qcapture', () => call('qoder_capture', { edition }))
}
async function qoderNote(a: any) {
  let value = a.note || ''
  Modal.open({
    title: t('账号备注', 'Account note'),
    content: () =>
      h('input', {
        class: 'arco-input arco-input-size-medium',
        style: 'width:100%;border:1px solid var(--line);border-radius:8px;padding:6px 10px',
        value,
        onInput: (e: any) => (value = e.target.value),
      }),
    onOk: () => act('qnote', () => call('qoder_set_note', { id: a.id, note: value })),
  })
}
function qoderRefreshToken(a: any) {
  return act(`qtoken:${a.id}`, () => call('qoder_refresh_account', { id: a.id }))
}
function qoderRemove(a: any) {
  Modal.confirm({
    title: t('删除账号', 'Delete account'),
    content: t(
      '从账号库删除该账号？（不影响客户端当前登录）',
      'Remove this account from the store? (the IDE stays signed in)'
    ),
    okButtonProps: { status: 'danger' },
    onOk: () => act('qdelete', () => call('qoder_delete_account', { id: a.id })),
  })
}

// ── Trae ──
function traeRun(a: any, cmd: string, msg: string, extra: Record<string, unknown> = {}) {
  return act(`trae:${a.id}:${cmd}`, () => call(cmd, { id: a.id, ...extra }), msg)
}
async function traeNote(a: any) {
  let value = a.note || ''
  Modal.open({
    title: t('账号备注', 'Account note'),
    content: () =>
      h('input', {
        class: 'arco-input',
        style: 'width:100%;border:1px solid var(--line);border-radius:8px;padding:6px 10px',
        value,
        onInput: (e: any) => (value = e.target.value),
      }),
    onOk: () =>
      traeRun(a, 'trae_update_account', t('已保存备注', 'Note saved'), { patch: { note: value } }),
  })
}
function traeSwitch(a: any) {
  Modal.confirm({
    title: t('切换 Trae 账号', 'Switch Trae account'),
    content: t(
      '如果对应的 Trae 客户端正在运行，会先自动退出（未响应时强制结束），再写入登录态并重新打开。未保存的编辑内容请先保存；原始登录存储会先备份。',
      'If the Trae client is running it is quit first (force-closed if it does not respond), then the login is written and the client reopens. Save open work first; the original login storage is backed up.'
    ),
    onOk: () =>
      traeRun(
        a,
        'trae_switch',
        t('账号已切换，客户端已重新打开（原始存储已备份）', 'Switched and reopened; original storage backed up')
      ),
  })
}
function traeRemove(a: any) {
  Modal.confirm({
    title: t('删除账号', 'Delete account'),
    content: t('从本机加密账号库删除此账号？', 'Remove this account from the local vault?'),
    okButtonProps: { status: 'danger' },
    onOk: () => act('tdelete', () => call('trae_delete', { id: a.id })),
  })
}
async function traeRefreshAll(ids: string[]) {
  await act(
    'trae-all',
    async () => {
      for (const id of ids) await call('trae_refresh', { id }).catch(() => {})
    },
    t('Trae 用量已刷新', 'Trae usage refreshed')
  )
}
/** Disabled accounts are left out of this batch; their cards can still refresh one by one. */
async function refreshAllCredits() {
  const active = (provider: string, list: any[]) => list.filter((a) => !isDisabled(provider, a.id)).map((a) => a.id as string)
  const wb = show('workbuddy') ? active('workbuddy', wbAccounts.value) : []
  const qoder = show('qoder') ? active('qoder', qoderAccounts.value) : []
  const trae = show('trae') ? active('trae', traeAccounts.value) : []
  if (!wb.length && !qoder.length && !trae.length) return Message.info(t('没有可刷新的账号（已禁用的账号不参与）', 'Nothing to refresh (disabled accounts are skipped)'))
  if (wb.length) await wbCredits(wb)
  if (qoder.length) await qoderCredits(qoder)
  if (trae.length) await traeRefreshAll(trae)
}
const clients = computed(() => state.pool?.workbuddy?.clients)
const qEditions = computed(() => state.pool?.qoder?.editions ?? {})
</script>

<template>
  <PageHead
    eyebrow="ACCOUNT POOL"
    :title="t('账号池', 'Accounts')"
    :desc="
      t(
        '三端、两个地区的账号统一管理；反代开关决定账号是否加入统一调度。',
        'All providers and regions in one pool; the proxy switch decides whether an account joins scheduling.'
      )
    "
  >
    <a-button @click="dialog = { kind: 'json-import', provider: 'auto' }">{{ t('导入 JSON', 'Import JSON') }}</a-button>
    <a-button :disabled="!exportAccounts.length" @click="dialog = { kind: 'json-export-batch' }">{{ t('批量导出', 'Batch export') }}</a-button>
    <a-button
      :loading="busy('wb-credits') || busy('qcredit:all') || busy('trae-all')"
      @click="refreshAllCredits"
      ><template #icon><IconRefresh /></template>{{ t('刷新积分', 'Refresh credits') }}</a-button
    >
    <a-dropdown trigger="click" position="br" :popup-max-height="560">
      <a-button type="primary"
        ><template #icon><IconPlus /></template>{{ t('添加账号', 'Add account') }}</a-button
      >
      <template #content>
        <a-dgroup title="WorkBuddy">
          <a-doption @click="dialog = { kind: 'wb-oauth' }">{{
            t('OAuth 登录添加', 'Add via OAuth')
          }}</a-doption>
          <a-doption @click="wbImport('local')">{{
            t('导入当前登录账号', 'Import signed-in account')
          }}</a-doption>
          <a-doption @click="wbImport('switch')">{{
            t('导入 WorkBuddy Switch 账号库', 'Import WorkBuddy Switch store')
          }}</a-doption>
          <a-doption @click="dialog = { kind: 'json-import', provider: 'workbuddy' }">{{
            t('导入账号 JSON', 'Import account JSON')
          }}</a-doption>
        </a-dgroup>
        <a-dgroup title="Qoder">
          <a-doption @click="dialog = { kind: 'login', provider: 'qoder', region: 'cn' }">{{
            t('浏览器登录 · 国内版', 'Browser sign-in · China')
          }}</a-doption>
          <a-doption @click="dialog = { kind: 'login', provider: 'qoder', region: 'intl' }">{{
            t('浏览器登录 · 国际版', 'Browser sign-in · International')
          }}</a-doption>
          <a-doption @click="qoderCapture('cn')">{{
            t('采集 Qoder CN 当前账号', 'Capture Qoder CN account')
          }}</a-doption>
          <a-doption @click="qoderCapture('intl')">{{
            t('采集 Qoder 当前账号', 'Capture Qoder account')
          }}</a-doption>
          <a-doption @click="dialog = { kind: 'json-import', provider: 'qoder' }">{{
            t('导入账号 JSON', 'Import account JSON')
          }}</a-doption>
        </a-dgroup>
        <a-dgroup title="Trae">
          <a-doption @click="dialog = { kind: 'login', provider: 'trae', region: 'cn' }">{{
            t('浏览器登录 · Trae CN', 'Browser sign-in · Trae CN')
          }}</a-doption>
          <a-doption @click="dialog = { kind: 'login', provider: 'trae', region: 'intl' }">{{
            t('浏览器登录 · Trae', 'Browser sign-in · Trae')
          }}</a-doption>
          <a-doption
            @click="
              act(
                'tcapture',
                () => call('trae_capture', { variant: 'cn' }),
                t('Trae CN 当前账号已采集', 'Trae CN account captured')
              )
            "
            >{{ t('采集 Trae CN 本机账号', 'Capture Trae CN account') }}</a-doption
          >
          <a-doption
            @click="
              act(
                'tcapture',
                () => call('trae_capture', { variant: 'intl' }),
                t('Trae 当前账号已采集', 'Trae account captured')
              )
            "
            >{{ t('采集 Trae 本机账号', 'Capture Trae account') }}</a-doption
          >
          <a-doption @click="dialog = { kind: 'json-import', provider: 'trae' }">{{
            t('导入账号 JSON', 'Import account JSON')
          }}</a-doption>
        </a-dgroup>
      </template>
    </a-dropdown>
  </PageHead>

  <div class="filter-bar">
    <a-radio-group v-model="provider" type="button">
      <a-radio value="all">{{ t('全部', 'All') }}</a-radio>
      <a-radio value="workbuddy">WorkBuddy</a-radio>
      <a-radio value="qoder">Qoder</a-radio>
      <a-radio value="trae">Trae</a-radio>
    </a-radio-group>
    <a-radio-group v-model="region" type="button">
      <a-radio value="all">{{ t('全部地区', 'All regions') }}</a-radio>
      <a-radio value="cn">{{ t('国内版', 'China') }}</a-radio>
      <a-radio value="intl">{{ t('国际版', 'International') }}</a-radio>
    </a-radio-group>
    <span class="grow"></span>
    <label class="reveal-switch" :title="t('仅本次运行有效；重启后恢复脱敏显示', 'This session only; masked again after restart')">
      <IconEye v-if="revealPhones" /><IconEyeInvisible v-else />{{ t('显示完整手机号', 'Show full numbers') }}
      <a-switch v-model="revealPhones" size="small" />
    </label>
    <span class="muted">{{ total }} {{ t('个账号', 'accounts') }}</span>
    <a-input-search
      v-model="query"
      :placeholder="
        t('搜索名称、备注、邮箱、手机号或 UID', 'Search name, note, email, phone or UID')
      "
      style="width: 280px"
      allow-clear
    />
    <a-button @click="compact = !compact">{{
      compact ? t('标准卡片', 'Standard') : t('紧凑卡片', 'Compact')
    }}</a-button>
  </div>

  <template v-if="show('workbuddy')">
    <div class="section-title">
      <ProviderMark provider="workbuddy" /><b>WorkBuddy</b
      ><span class="muted">{{ wbAccounts.length }}</span
      ><span class="grow"></span>
      <a-button
        size="small"
        :loading="busy('wb-local')"
        @click="
          act(
            'wb-local',
            () => call('wb_refresh_statuses'),
            t('已更新本机模型限额与客户端状态', 'Local limits and clients refreshed')
          )
        "
        ><template #icon><IconThunderbolt /></template
        >{{ t('刷新限额状态', 'Refresh limits') }}</a-button
      >
    </div>
    <div v-if="!wbAccounts.length" class="empty-state">
      <strong>{{ t('还没有 WorkBuddy 账号', 'No WorkBuddy accounts') }}</strong
      >{{
        t(
          '通过 OAuth 登录、导入当前登录账号或 WorkBuddy Switch 账号库添加。',
          'Add via OAuth, the signed-in account, or a WorkBuddy Switch store.'
        )
      }}
    </div>
    <div v-else class="switch-account-grid" :class="{ compact }">
      <AccountCard
        v-for="a in wbAccounts"
        :key="a.id"
        :account="a"
        :clients="clients"
        :now="state.now"
        :compact="compact"
        :busy="busy(`wb:${a.id}`) || busy('wb-switch')"
        :credit-loading="creditLoading.includes(a.id)"
        @enable="(v) => wbEnable(a, v)"
        @checkin-enable="(v) => wbCheckinEnable(a, v)"
        @checkin="wbCheckin(a)"
        @refresh="wbCredits([a.id])"
        @info="dialog = { kind: 'wb-info', id: a.id }"
        @resources="dialog = { kind: 'wb-resources', id: a.id }"
        @switch="(target) => wbSwitch(a, target)"
        @json="dialog = { kind: 'json-view', provider: 'workbuddy', id: a.id, name: accountName(a) }"
        @relogin="dialog = { kind: 'wb-oauth', variant: a.variant, accountId: a.id }"
        @renew="wbRenew(a)"
      />
    </div>
  </template>

  <template v-if="show('qoder')">
    <div class="section-title">
      <ProviderMark provider="qoder" /><b>Qoder</b
      ><span class="muted">{{ qoderAccounts.length }}</span>
      <span v-for="(ed, key) in qEditions" :key="key" class="rtag"
        >{{ key === 'cn' ? 'Qoder CN' : 'Qoder' }} ·
        {{
          ed.installed
            ? ed.running
              ? t('运行中', 'running')
              : t('未运行', 'not running')
            : t('未安装', 'not installed')
        }}{{ ed.currentName ? ' · ' + ed.currentName : '' }}</span
      >
      <span class="grow"></span>
      <a-button
        v-for="(ed, key) in qEditions"
        :key="'open' + key"
        size="small"
        :disabled="!ed.installed"
        @click="act('qopen', () => call('qoder_open_app', { edition: key }))"
        ><template #icon><IconLaunch /></template
        >{{
          key === 'cn' ? t('打开 Qoder CN', 'Open Qoder CN') : t('打开 Qoder', 'Open Qoder')
        }}</a-button
      >
    </div>
    <div v-if="!qoderAccounts.length" class="empty-state">
      <strong>{{ t('还没有 Qoder 账号', 'No Qoder accounts') }}</strong
      >{{
        t(
          '使用浏览器登录添加，或先在客户端登录后采集当前账号。',
          'Sign in via browser, or capture the account signed in to the client.'
        )
      }}
    </div>
    <div v-else class="switch-account-grid" :class="{ compact }">
      <QoderCard
        v-for="a in qoderAccounts"
        :key="a.id"
        :account="a"
        :compact="compact"
        @switch="dialog = { kind: 'qoder-switch', account: a }"
        @claim="qoderClaim(a)"
        @credits="qoderCredits([a.id])"
        @info="dialog = { kind: 'qoder-info', account: a }"
        @note="qoderNote(a)"
        @refresh-token="qoderRefreshToken(a)"
        @remove="qoderRemove(a)"
        @json="dialog = { kind: 'json-view', provider: 'qoder', id: a.id, name: a.note || a.name || a.email }"
      />
    </div>
  </template>

  <template v-if="show('trae')">
    <div class="section-title">
      <ProviderMark provider="trae" /><b>Trae</b
      ><span class="muted">{{ traeAccounts.length }}</span>
      <span v-for="c in state.pool?.trae?.clients ?? []" :key="c.variant" class="rtag"
        >{{ c.name }} ·
        {{
          c.installed
            ? c.version
              ? 'v' + c.version
              : t('已安装', 'installed')
            : t('未安装', 'not installed')
        }}</span
      >
      <span class="grow"></span>
      <a-button
        v-for="c in state.pool?.trae?.clients ?? []"
        :key="'open' + c.variant"
        size="small"
        :disabled="!c.installed"
        @click="act('topen', () => call('trae_launch', { variant: c.variant }))"
        ><template #icon><IconLaunch /></template>{{ t('打开 ', 'Open ') + c.name }}</a-button
      >
    </div>
    <div v-if="!traeAccounts.length" class="empty-state">
      <strong>{{ t('还没有 Trae 账号', 'No Trae accounts') }}</strong
      >{{
        t(
          '从本机采集已登录账号，或使用浏览器独立登录。',
          'Capture a signed-in account or sign in via browser.'
        )
      }}
    </div>
    <div v-else class="switch-account-grid" :class="{ compact }">
      <TraeCard
        v-for="a in traeAccounts"
        :key="a.id"
        :account="a"
        :compact="compact"
        @refresh="traeRun(a, 'trae_refresh', t('用量刷新完成', 'Usage refreshed'))"
        @switch="traeSwitch(a)"
        @checkin="traeRun(a, 'trae_checkin', t('签到状态已确认', 'Check-in confirmed'))"
        @auto-checkin="
          (v) =>
            traeRun(
              a,
              'trae_update_account',
              v
                ? t('自动签到已开启', 'Auto check-in on')
                : t('自动签到已关闭', 'Auto check-in off'),
              { patch: { autoCheckin: v } }
            )
        "
        @packs="dialog = { kind: 'trae-packs', account: a }"
        @note="traeNote(a)"
        @token="traeRun(a, 'trae_refresh_token', t('登录凭证已刷新', 'Login refreshed'))"
        @models="traeRun(a, 'trae_sync_models', t('模型目录已同步', 'Models synced'))"
        @remove="traeRemove(a)"
        @json="dialog = { kind: 'json-view', provider: 'trae', id: a.id, name: a.note || a.username }"
      />
    </div>
    <div class="notice" style="margin-top: 14px">
      {{
        t(
          'Trae 切换前自动备份原始登录态；客户端运行中会先自动退出（必要时强制结束），切换成功后重新打开。',
          'Trae switching backs up the original login; a running client is quit first (forced if needed) and reopens afterwards.'
        )
      }}
    </div>
  </template>

  <AddAccountDialog
    v-if="dialog?.kind === 'wb-oauth'"
    :initial-variant="dialog.variant"
    :expected-account-id="dialog.accountId"
    @close="dialog = null"
    @added="refreshPool"
  />
  <WbDrawers
    v-if="dialog?.kind === 'wb-info' || dialog?.kind === 'wb-resources'"
    :mode="dialog.kind === 'wb-info' ? 'info' : 'resources'"
    :account="state.pool?.workbuddy?.accounts.find((a: any) => a.id === dialog!.id)"
    @close="dialog = null"
  />
  <QoderSwitchDialog
    v-if="dialog?.kind === 'qoder-switch'"
    :account="dialog.account"
    @close="dialog = null"
  />
  <QoderInfoDialog
    v-if="dialog?.kind === 'qoder-info'"
    :account="dialog.account"
    @close="dialog = null"
  />
  <LoginDialog
    v-if="dialog?.kind === 'login'"
    :provider="dialog.provider"
    :region="dialog.region"
    @close="dialog = null"
  />
  <TraeDialogs
    v-if="dialog?.kind === 'trae-packs'"
    mode="packs"
    :account="dialog.account"
    @close="dialog = null"
  />
  <AccountJsonDialog
    v-if="dialog?.kind === 'json-view' || dialog?.kind === 'json-import'"
    :mode="dialog.kind === 'json-view' ? 'view' : 'import'"
    :provider="dialog.provider"
    :id="dialog.id"
    :name="dialog.name"
    @close="dialog = null"
  />
  <AccountBatchExportDialog v-if="dialog?.kind === 'json-export-batch'" :accounts="exportAccounts" @close="dialog = null" />
</template>
