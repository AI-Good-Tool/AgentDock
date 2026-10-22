<script setup lang="ts">
// WorkBuddy local sessions (WorkBuddy Switch session core): per-account list,
// copy to another account (CN ⇄ International), linked groups with safe sync.
import { computed, ref, watch, onMounted } from 'vue'
import { Message, Modal } from '@arco-design/web-vue'
import { IconCopy, IconSync, IconLink, IconRefresh } from '@arco-design/web-vue/es/icon'
import { call } from '../api'
import { t, fmtShort } from '../i18n'

type Variant = 'cn' | 'ai'
const variant = ref<Variant>('cn')
const accounts = ref<{ variant: Variant; current: string | null; running: boolean; accounts: any[] }>({ variant: 'cn', current: null, running: false, accounts: [] })
const uid = ref('')
const sessions = ref<any[]>([])
const groups = ref<any>({ groups: [], storeStatus: 'missing' })
const query = ref('')
const kind = ref<'all' | 'space' | 'task'>('all')
const selected = ref(new Set<string>())
const loading = ref(false)
const groupsLoading = ref(false)
const working = ref('')
const openGroup = ref<string | null>(null)
const groupDetail = ref<any>(null)

async function loadAccounts() {
  accounts.value = await call('wb_session_accounts', { variant: variant.value }).catch(() => ({ variant: variant.value, current: null, running: false, accounts: [] }))
  const list = accounts.value.accounts
  if (!list.some((a) => a.uid === uid.value)) uid.value = accounts.value.current || list[0]?.uid || ''
}
async function loadSessions() {
  selected.value = new Set()
  sessions.value = uid.value ? ((await call('wb_list_sessions', { variant: variant.value, uid: uid.value }).catch(() => ({ sessions: [] })))?.sessions ?? []) : []
}
// Group status hashes every member's history (a few seconds on large stores), so it never blocks the list.
async function loadGroups() {
  groupsLoading.value = true
  try {
    groups.value = await call('wb_session_groups').catch((e) => ({ groups: [], storeStatus: 'unavailable', storeError: String(e) }))
  } finally {
    groupsLoading.value = false
  }
}
async function load() {
  loading.value = true
  loadGroups()
  try {
    await loadAccounts()
    await loadSessions()
  } finally {
    loading.value = false
  }
}
onMounted(load)
watch(variant, async () => {
  loading.value = true
  try {
    await loadAccounts()
    await loadSessions()
  } finally {
    loading.value = false
  }
})
watch(uid, loadSessions)
defineExpose({ load })

const accountName = (u: string) => accounts.value.accounts.find((a) => a.uid === u)?.name || u
const filtered = computed(() => {
  const q = query.value.trim().toLowerCase()
  return sessions.value.filter(
    (s) =>
      (kind.value === 'all' || (kind.value === 'task' ? s.isPlayground : !s.isPlayground)) &&
      (!q || `${s.title} ${s.cwd}`.toLowerCase().includes(q))
  )
})
const project = (cwd: string) => (cwd || '').replace(/[\\/]+$/, '').split(/[\\/]/).pop() || '—'
function toggle(id: string) {
  const next = new Set(selected.value)
  next.has(id) ? next.delete(id) : next.add(id)
  selected.value = next
}
const allSelected = computed(() => filtered.value.length > 0 && filtered.value.every((s) => selected.value.has(s.id)))
function toggleAll() {
  selected.value = allSelected.value ? new Set() : new Set(filtered.value.map((s) => s.id))
}

// Copy dialog
const copyOpen = ref(false)
const targetVariant = ref<Variant>('cn')
const targetUid = ref('')
const targetList = ref<any>({ accounts: [], running: false })
async function loadTargets() {
  targetList.value = await call('wb_session_accounts', { variant: targetVariant.value }).catch(() => ({ accounts: [], running: false }))
  const options = targetList.value.accounts.filter((a: any) => !(targetVariant.value === variant.value && a.uid === uid.value))
  if (!options.some((a: any) => a.uid === targetUid.value)) targetUid.value = options[0]?.uid || ''
}
const targetOptions = computed(() => (targetList.value.accounts ?? []).filter((a: any) => !(targetVariant.value === variant.value && a.uid === uid.value)))
watch(targetVariant, loadTargets)
function openCopy(ids?: string[]) {
  if (ids) selected.value = new Set(ids)
  if (!selected.value.size) return Message.warning(t('请先选择会话', 'Select sessions first'))
  targetVariant.value = variant.value
  copyOpen.value = true
  loadTargets()
}
async function doCopy() {
  if (!targetUid.value) {
    Message.error(t('请选择目标账号', 'Choose a target account'))
    return false
  }
  working.value = 'copy'
  try {
    const r = await call('wb_copy_sessions', {
      variant: variant.value,
      uid: uid.value,
      targetVariant: targetVariant.value,
      targetUid: targetUid.value,
      sessionIds: [...selected.value],
      quit: true,
    })
    const copied = r?.copied?.length ?? 0
    const linked = r?.alreadyLinked?.length ?? 0
    const failed = r?.errors?.length ?? 0
    const text = t(
      `已复制 ${copied} 个${linked ? `，${linked} 个已有关联副本` : ''}${failed ? `，${failed} 个失败：${r.errors[0].error}` : ''}`,
      `${copied} copied${linked ? `, ${linked} already linked` : ''}${failed ? `, ${failed} failed: ${r.errors[0].error}` : ''}`
    )
    failed ? Message.warning(text) : Message.success(text)
    copyOpen.value = false
    selected.value = new Set()
    loadGroups()
    return true
  } catch (e) {
    Message.error(String(e))
    return false
  } finally {
    working.value = ''
  }
}

// Linked groups
const summaryClass = (s: string) => (s === 'latest' ? '' : s === 'behind' ? 'warn' : 'bad')
const summaryName: Record<string, [string, string]> = {
  latest: ['内容一致', 'In sync'],
  behind: ['可安全同步', 'Can sync'],
  diverge: ['已分叉', 'Diverged'],
  missing: ['内容缺失', 'Missing'],
  stale: ['已失效', 'Stale'],
  superseded: ['已被替代', 'Superseded'],
  unknown: ['无法确认', 'Unknown'],
}
const statusText = (s: string) => (summaryName[s] ? t(...summaryName[s]) : s)
async function toggleGroup(id: string) {
  if (openGroup.value === id) {
    openGroup.value = null
    return
  }
  openGroup.value = id
  groupDetail.value = null
  groupDetail.value = await call('wb_session_group_detail', { groupId: id }).catch((e) => ({ error: String(e) }))
}
async function syncGroup(g: any) {
  working.value = 'sync:' + g.groupId
  try {
    const r = await call('wb_sync_session_group', { groupId: g.groupId })
    const synced = r?.synced?.length ?? 0
    const errors = r?.errors?.length ?? 0
    errors ? Message.warning(t(`同步 ${synced} 个，失败 ${errors} 个`, `${synced} synced, ${errors} failed`)) : Message.success(t(`已同步 ${synced} 个副本`, `${synced} copies synced`))
  } catch (e) {
    Message.error(String(e))
  } finally {
    working.value = ''
    loadGroups()
    if (openGroup.value === g.groupId) toggleGroup(g.groupId).then(() => toggleGroup(g.groupId))
  }
}
function unlink(g: any) {
  Modal.confirm({
    title: t('解除关联', 'Unlink group'),
    content: t('只解除副本之间的关联，各账号内的会话内容不会被删除。', 'Only the link is removed; sessions in each account are kept.'),
    onOk: async () => {
      try {
        await call('wb_unlink_session_group', { groupId: g.groupId })
        Message.success(t('已解除关联', 'Unlinked'))
      } catch (e) {
        Message.error(String(e))
      }
      loadGroups()
    },
  })
}
const variantLabel = (v: string) => (v === 'ai' ? t('国际版', 'International') : t('国内版', 'China'))
</script>

<template>
  <div class="filter-bar">
    <a-radio-group v-model="variant" type="button">
      <a-radio value="cn">WorkBuddy {{ t('国内版', 'CN') }}</a-radio>
      <a-radio value="ai">WorkBuddy {{ t('国际版', 'Intl') }}</a-radio>
    </a-radio-group>
    <a-select v-model="uid" style="width: 240px" :placeholder="t('选择账号', 'Choose account')" :disabled="!accounts.accounts.length">
      <a-option v-for="a in accounts.accounts" :key="a.uid" :value="a.uid">{{ a.name }}{{ a.current ? t('（当前登录）', ' (signed in)') : '' }}</a-option>
    </a-select>
    <a-input-search v-model="query" :placeholder="t('搜索标题或项目路径…', 'Search title or project path…')" style="width: 260px" allow-clear />
    <span v-if="accounts.running" class="state-pill warn">{{ t('WorkBuddy 运行中 · 复制时会自动退出', 'WorkBuddy running · it is closed before copying') }}</span>
  </div>

  <section class="box">
    <div class="box-head">
      <h2>{{ t('关联会话', 'Linked groups') }} <span class="rtag">{{ groups.groups.length }}</span></h2>
      <a-button size="small" :loading="groupsLoading" @click="loadGroups"><template #icon><IconRefresh /></template>{{ t('刷新', 'Refresh') }}</a-button>
    </div>
    <a-alert v-if="groups.storeStatus === 'unavailable'" type="warning" style="margin-bottom: 10px">{{ groups.storeError }}</a-alert>
    <div v-if="groupsLoading && !groups.groups.length" class="muted"><a-spin :size="14" /> {{ t('正在核对关联副本内容…', 'Checking linked copies…') }}</div>
    <div v-else-if="!groups.groups.length" class="muted">{{ t('暂无关联组 —— 把会话复制到其他账号后自动建立关联，之后可在这里同步最新内容', 'No linked groups yet — copying a session to another account links them so you can sync later') }}</div>
    <div v-for="g in groups.groups" :key="g.groupId" class="wb-group">
      <div class="wb-group-row" @click="toggleGroup(g.groupId)">
        <div class="wb-group-main">
          <b>{{ g.title || t('(无标题)', '(Untitled)') }}</b>
          <small>{{ g.projectLabel || '—' }} · {{ (g.accountNames || []).join('、') }} · {{ g.memberCount }} {{ t('个副本', 'copies') }}<template v-if="g.latestActivityAt"> · {{ fmtShort(g.latestActivityAt) }}</template></small>
        </div>
        <span class="state-pill" :class="summaryClass(g.summaryStatus)" :title="g.summaryText">{{ statusText(g.summaryStatus) }}</span>
        <a-button size="mini" type="primary" :disabled="!g.hasSafeSource || g.summaryStatus !== 'behind'" :loading="working === 'sync:' + g.groupId" @click.stop="syncGroup(g)"><template #icon><IconSync /></template>{{ t('同步', 'Sync') }}</a-button>
        <a-button size="mini" @click.stop="unlink(g)"><template #icon><IconLink /></template>{{ t('解除关联', 'Unlink') }}</a-button>
      </div>
      <div v-if="openGroup === g.groupId" class="wb-group-detail">
        <a-spin v-if="!groupDetail" />
        <div v-else-if="groupDetail.error" class="error-text">{{ groupDetail.error }}</div>
        <div v-for="m in groupDetail?.members || []" v-else :key="m.memberId" class="kv">
          <span>[{{ variantLabel(m.variant) }}] {{ m.accountName || m.uid }} · {{ m.title || m.sessionId?.slice(0, 8) }}</span>
          <b>
            <span class="state-pill" :class="{ warn: m.versionStatus === 'behind', bad: ['diverge', 'missing'].includes(m.versionStatus), off: ['stale', 'superseded', 'unknown'].includes(m.versionStatus) }">{{ statusText(m.versionStatus) }}</span>
            <small class="muted"> {{ m.recordCount ?? '—' }} {{ t('条记录', 'records') }}<template v-if="m.updatedAt"> · {{ fmtShort(m.updatedAt) }}</template></small>
          </b>
        </div>
      </div>
    </div>
  </section>

  <div class="filter-bar">
    <a-radio-group v-model="kind" type="button" size="small">
      <a-radio value="all">{{ t('全部', 'All') }} {{ sessions.length }}</a-radio>
      <a-radio value="space">{{ t('空间', 'Spaces') }} {{ sessions.filter((s) => !s.isPlayground).length }}</a-radio>
      <a-radio value="task">{{ t('任务', 'Tasks') }} {{ sessions.filter((s) => s.isPlayground).length }}</a-radio>
    </a-radio-group>
    <a-checkbox :model-value="allSelected" :disabled="!filtered.length" @change="toggleAll">{{ t('全选', 'Select all') }}</a-checkbox>
    <span class="muted">{{ t(`已选 ${selected.size} 个`, `${selected.size} selected`) }}</span>
    <span style="flex: 1"></span>
    <a-button type="primary" :disabled="!selected.size" @click="openCopy()"><template #icon><IconCopy /></template>{{ t('复制到其他账号', 'Copy to account') }}</a-button>
  </div>
  <div v-if="!accounts.accounts.length && !loading" class="empty-state">{{ t('没有可用账号：请在账号池导入 WorkBuddy 账号，或在 WorkBuddy 客户端登录', 'No accounts: import WorkBuddy accounts in Accounts or sign in to the WorkBuddy client') }}</div>
  <div v-else-if="!filtered.length && !loading" class="empty-state">{{ t('该账号下没有符合条件的会话', 'No sessions match for this account') }}</div>
  <div class="session-grid">
    <div v-for="s in filtered" :key="s.id" class="session-card" :class="{ picked: selected.has(s.id) }" @click="toggle(s.id)">
      <small class="mono session-project"><a-checkbox :model-value="selected.has(s.id)" @click.stop @change="toggle(s.id)" />{{ s.isPlayground ? t('任务', 'Task') : project(s.cwd) }}</small>
      <strong :title="s.title">{{ s.title || t('(无标题)', '(Untitled)') }}</strong>
      <small>{{ fmtShort(s.updatedAt) }}<template v-if="!s.hasHistory"> · {{ t('无本地正文', 'no local history') }}</template></small>
      <div style="display: flex; gap: 6px">
        <a-button size="mini" @click.stop="openCopy([s.id])"><template #icon><IconCopy /></template>{{ t('复制到…', 'Copy to…') }}</a-button>
      </div>
    </div>
  </div>

  <a-modal v-model:visible="copyOpen" :title="t('复制会话到其他账号', 'Copy sessions to another account')" :ok-loading="working === 'copy'" :ok-text="t('复制', 'Copy')" :on-before-ok="doCopy">
    <div class="copy-form">
      <div class="kv"><span>{{ t('来源', 'From') }}</span><b>[{{ variantLabel(variant) }}] {{ accountName(uid) }} · {{ selected.size }} {{ t('个会话', 'sessions') }}</b></div>
      <div class="copy-line">
        <span class="muted">{{ t('目标版本', 'Target version') }}</span>
        <a-radio-group v-model="targetVariant" type="button" size="small">
          <a-radio value="cn">{{ t('国内版', 'China') }}</a-radio>
          <a-radio value="ai">{{ t('国际版', 'International') }}</a-radio>
        </a-radio-group>
      </div>
      <div class="copy-line">
        <span class="muted">{{ t('目标账号', 'Target account') }}</span>
        <a-select v-model="targetUid" style="flex: 1" :placeholder="t('选择账号', 'Choose account')">
          <a-option v-for="a in targetOptions" :key="a.uid" :value="a.uid">{{ a.name }}{{ a.current ? t('（当前登录）', ' (signed in)') : '' }}</a-option>
        </a-select>
      </div>
      <p class="hint">
        {{ t('生成新的会话 ID 写入目标账号（正文与会话记录），并与原会话建立关联，之后可同步最新内容。写入前自动备份；目标版本的 WorkBuddy 正在运行时会先退出。目标账号下次打开 WorkBuddy 时由客户端上传到云端。', 'Writes copies with new IDs into the target account (history and session record) and links them to the originals for later sync. Backed up first; a running WorkBuddy of the target version is closed. The client uploads them the next time the target account opens WorkBuddy.') }}
      </p>
    </div>
  </a-modal>
</template>

<style>
.wb-group {
  border-bottom: 1px solid var(--line);
}
.wb-group:last-child {
  border-bottom: none;
}
.wb-group-row {
  display: flex;
  align-items: center;
  gap: 10px;
  padding: 10px 0;
  cursor: pointer;
}
.wb-group-main {
  flex: 1;
  min-width: 0;
}
.wb-group-main b {
  display: block;
  font-size: 13.5px;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.wb-group-main small {
  color: var(--muted);
  font-size: 12px;
}
.wb-group-detail {
  padding: 0 0 10px 14px;
}
.session-card.picked {
  border-color: var(--accent);
  box-shadow: 0 0 0 1px var(--accent);
}
.session-card {
  cursor: pointer;
}
.session-project {
  display: flex;
  align-items: center;
  gap: 4px;
}
.copy-form {
  display: flex;
  flex-direction: column;
  gap: 12px;
}
.copy-line {
  display: flex;
  align-items: center;
  gap: 12px;
}
.copy-line > .muted {
  width: 64px;
  flex: none;
}
</style>
