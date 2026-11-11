<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref } from 'vue'
import { listen, type UnlistenFn } from '@tauri-apps/api/event'
import {
  IconSwap,
  IconLink,
  IconCopy,
  IconRefresh,
  IconCheckCircle,
  IconArrowRight,
} from '@arco-design/web-vue/es/icon'
import { call, desktop } from '../../api'
import { state, refreshPool } from '../../store'
import { accountName } from '../../lib/account-display'
import {
  canCopySession,
  directSync,
  syncMode,
  syncSelections,
  sessionKey,
  migrationIssues,
  type LinkPreview,
} from '../../lib/wb-switch'
import { t, fmtShort } from '../../i18n'

const props = defineProps<{ account: any; target: string }>()
const emit = defineEmits<{ close: [] }>()
const client = computed(
  () =>
    ({ workbuddy: 'WorkBuddy', ide: 'CodeBuddy IDE', vscode: 'VS Code', cli: 'CodeBuddy CLI' })[
      props.target
    ] || props.target
)
const tab = ref('links')
const query = ref('')
const loading = ref(true)
const working = ref(false)
const error = ref('')
const progress = ref('')
const preview = ref<any>({})
const groups = computed<LinkPreview[]>(() => preview.value.groups || [])
const sessions = computed<any[]>(() => preview.value.sessions || [])
const checked = ref(new Set<string>())
const copied = ref(new Set<string>())
const result = ref<any>(null)
const issues = computed(() =>
  result.value
    ? migrationIssues(
        result.value,
        t(
          '会话写入尚未完成，需要恢复；请重新检查后重试',
          'Session writes need recovery; recheck before retrying'
        )
      )
    : []
)
const detail = ref(new Set<string>())
const direct = computed(() => groups.value.filter(directSync))
const selected = computed(() => syncSelections(groups.value, checked.value))
const filtered = computed(() =>
  sessions.value.filter((s) =>
    `${s.title} ${s.cwd || s.workspaceHash || ''}`
      .toLowerCase()
      .includes(query.value.trim().toLowerCase())
  )
)
const copyable = computed(() => filtered.value.filter((s) => canCopySession(preview.value, s)))
const allDirect = computed(
  () => direct.value.length > 0 && direct.value.every((g) => checked.value.has(g.groupId))
)
const allCopy = computed(
  () => copyable.value.length > 0 && copyable.value.every((s) => copied.value.has(sessionKey(s)))
)
const labels: Record<string, [string, string]> = {
  fastForward: ['可直接同步', 'Ready to sync'],
  ahead: ['目标账号有更新', 'Target is newer'],
  diverge: ['双方都有更新', 'Both changed'],
  identical: ['内容一致', 'Identical'],
  unknown: ['无法确认', 'Unknown'],
}
const verdict = (g: LinkPreview) => (labels[g.verdict] ? t(...labels[g.verdict]!) : g.verdict)
function toggle(set: Set<string>, key: string) {
  set.has(key) ? set.delete(key) : set.add(key)
}
function toggleDirect() {
  const next = new Set(checked.value)
  direct.value.forEach((g) => (allDirect.value ? next.delete(g.groupId) : next.add(g.groupId)))
  checked.value = next
}
function toggleCopies() {
  const next = new Set(copied.value)
  copyable.value.forEach((s) =>
    allCopy.value ? next.delete(sessionKey(s)) : next.add(sessionKey(s))
  )
  copied.value = next
}
async function load() {
  loading.value = true
  error.value = ''
  checked.value = new Set()
  copied.value = new Set()
  preview.value = {}
  try {
    preview.value = await call('wb_switch_preview', { id: props.account.id, target: props.target })
    checked.value = new Set(
      groups.value.filter((g) => g.defaultChecked && directSync(g)).map((g) => g.groupId)
    )
  } catch (e) {
    error.value = String(e)
  } finally {
    loading.value = false
  }
}
let unlisten: UnlistenFn | undefined
let disposed = false
onMounted(async () => {
  void load()
  if (desktop) {
    const stop = await listen<{ message: string }>('wb-switch-progress', (e) => {
      if (working.value) progress.value = e.payload.message
    })
    if (disposed) stop()
    else unlisten = stop
  }
})
onUnmounted(() => {
  disposed = true
  unlisten?.()
})
function close() {
  if (!working.value) emit('close')
}
async function confirm() {
  if (working.value || state.pending.has('wb-switch')) return
  working.value = true
  state.pending.add('wb-switch')
  error.value = ''
  progress.value = t('正在备份登录态并切换客户端…', 'Backing up login and switching client…')
  try {
    const copies = sessions.value.filter(
      (s) => copied.value.has(sessionKey(s)) && canCopySession(preview.value, s)
    )
    result.value = await call('wb_switch_client', {
      id: props.account.id,
      target: props.target,
      confirmed: true,
      copySessionIds: props.target === 'workbuddy' ? copies.map((s) => s.id) : [],
      copyItems:
        props.target !== 'workbuddy'
          ? copies.map((s) => ({ workspaceHash: s.workspaceHash, conversationId: s.id }))
          : [],
      syncSelections: selected.value,
    })
    if (copies.length && !result.value.sessionCopy)
      result.value.sessionCopy = {
        error: t(
          '客户端已切换，但未收到会话复制结果',
          'Client switched, but no copy report was returned'
        ),
      }
    if (selected.value.length && !result.value.sessionSync)
      result.value.sessionSync = {
        error: t(
          '客户端已切换，但未收到会话同步结果',
          'Client switched, but no sync report was returned'
        ),
      }
    try {
      await refreshPool()
    } catch (e) {
      error.value =
        t('切换已完成，账号状态刷新失败：', 'Switched; account refresh failed: ') + String(e)
    }
  } catch (e) {
    error.value = String(e)
  } finally {
    working.value = false
    state.pending.delete('wb-switch')
  }
}
</script>

<template>
  <a-modal
    :visible="true"
    :width="760"
    :footer="false"
    :closable="!working"
    :mask-closable="false"
    :esc-to-close="!working"
    modal-class="wb-switch-modal"
    @cancel="close"
  >
    <template #title
      ><span class="ws-title"
        ><IconSwap />
        {{
          result ? t('切换结果', 'Switch result') : t('切换客户端账号', 'Switch client account')
        }}</span
      ></template
    >
    <div class="ws-content">
      <div class="ws-route">
        <div class="ws-client-icon">
          {{ target === 'cli' ? '>_' : target === 'vscode' ? '‹›' : target === 'ide' ? 'I' : 'W' }}
        </div>
        <div>
          <small
            >{{ client }} ·
            {{
              account.variant === 'ai' ? t('国际版', 'International') : t('国内版', 'China')
            }}</small
          ><strong>{{ accountName(account) }}</strong>
        </div>
        <span class="ws-route-label"><IconArrowRight /> {{ t('目标账号', 'Target account') }}</span>
      </div>
      <p class="ws-notice">
        {{
          target === 'cli'
            ? t(
                '切换会备份并改写 CLI 登录态，现有 CLI 会话可能中断，请先保存工作。',
                'The CLI login is backed up and replaced. Existing CLI sessions may end; save your work first.'
              )
            : t(
                '切换时会先关闭客户端，完成会话迁移与登录切换后重新打开。请先保存工作。',
                'The client closes before migration and login switching, then reopens. Save your work first.'
              )
        }}
      </p>
      <a-alert v-if="error" type="error" class="ws-alert">{{ error }}</a-alert>
      <template v-if="result">
        <div class="ws-result">
          <IconCheckCircle />
          <h3>{{ t('客户端账号已切换', 'Client account switched') }}</h3>
          <p>
            {{ t('已复制', 'Copied') }} {{ result.sessionCopy?.copied?.length || 0 }} ·
            {{ t('已有副本', 'Already linked') }}
            {{ result.sessionCopy?.alreadyLinked?.length || 0 }} · {{ t('已同步', 'Synced') }}
            {{ result.sessionSync?.synced?.length || 0 }}
          </p>
        </div>
        <a-alert v-if="issues.length" type="warning" class="ws-alert"
          ><b>{{ t('以下会话操作需要关注', 'Session operations need attention') }}</b>
          <p v-for="(issue, i) in issues" :key="i">{{ issue }}</p></a-alert
        >
        <p v-if="result.backup" class="ws-path">
          {{ t('登录态备份：', 'Login backup: ') }}{{ result.backup }}
        </p>
      </template>
      <template v-else-if="target !== 'cli'">
        <a-alert
          v-if="preview.previewError || preview.storeError || preview.supported === false"
          type="warning"
          class="ws-alert"
          >{{
            preview.previewError ||
            preview.storeError ||
            t(
              '当前客户端版本不支持会话迁移',
              'This client version does not support session migration'
            )
          }}</a-alert
        >
        <a-alert v-if="!loading && !error && !preview.sourceUid" type="info" class="ws-alert">{{
          t(
            '未读取到当前登录账号，暂时无法选择会话；可以仅切换账号。',
            'No current login was found. You can switch accounts without migrating sessions.'
          )
        }}</a-alert>
        <div class="ws-tabs" role="tablist">
          <button
            :class="{ active: tab === 'links' }"
            role="tab"
            :aria-selected="tab === 'links'"
            @click="tab = 'links'"
          >
            <IconLink /> {{ t('关联会话', 'Linked sessions') }} <span>{{ groups.length }}</span>
          </button>
          <button
            :class="{ active: tab === 'copy' }"
            role="tab"
            :aria-selected="tab === 'copy'"
            @click="tab = 'copy'"
          >
            <IconCopy /> {{ t('复制会话', 'Copy sessions') }} <span>{{ sessions.length }}</span>
          </button>
          <a-button size="small" :disabled="working || loading" :loading="loading" @click="load"
            ><template #icon><IconRefresh /></template>{{ t('重新检查', 'Recheck') }}</a-button
          >
        </div>
        <div v-if="loading" class="ws-empty">
          <a-spin />
          <p>
            {{ t('正在检查本地会话与关联副本…', 'Checking local sessions and linked copies…') }}
          </p>
        </div>
        <div v-else-if="preview.sameAccount" class="ws-empty">
          {{
            t(
              '此客户端已登录该账号，无需迁移会话。',
              'This client is already signed in to this account.'
            )
          }}
        </div>
        <template v-else-if="tab === 'links'">
          <div class="ws-info">
            <IconLink />
            <div>
              <b>{{ t('接着之前的对话继续', 'Continue your conversations') }}</b>
              <p>
                {{
                  t(
                    '复制过的会话会自动关联。可将当前账号的新内容同步到目标副本；双方都有更新时，需单独勾选覆盖。',
                    'Copied sessions are linked automatically. Sync new content into the target copy; overwriting conflicts requires an individual selection.'
                  )
                }}
              </p>
            </div>
          </div>
          <div class="ws-toolbar">
            <a-checkbox
              :model-value="allDirect"
              :disabled="working || !direct.length"
              @change="toggleDirect"
              >{{ t('全选可直接同步的会话', 'Select safe updates') }}</a-checkbox
            ><small>{{ t('已选', 'Selected') }} {{ selected.length }} / {{ groups.length }}</small>
          </div>
          <div class="ws-list" role="tabpanel">
            <a-empty
              v-if="!groups.length"
              :description="
                t(
                  '暂无关联会话，可前往「复制会话」创建副本',
                  'No linked sessions. Use Copy sessions to create copies.'
                )
              "
            />
            <div
              v-for="g in groups"
              :key="g.groupId"
              class="ws-session"
              :class="{ picked: checked.has(g.groupId) }"
            >
              <div class="ws-session-row">
                <a-checkbox
                  :model-value="checked.has(g.groupId)"
                  :disabled="working || !syncMode(g)"
                  :aria-label="g.title"
                  @change="toggle(checked, g.groupId)"
                />
                <div class="ws-session-main">
                  <strong>{{ g.title || t('无标题', 'Untitled') }}</strong>
                  <p>
                    {{
                      syncMode(g) === 'statusOnly'
                        ? t('仅同步归档，保留正文', 'Sync archive status only')
                        : g.verdict === 'fastForward'
                          ? t('同步当前账号的新内容', 'Sync new content from this account')
                          : g.verdict === 'diverge'
                            ? t('勾选后将覆盖目标会话全文', 'Selecting replaces the target history')
                            : g.verdict === 'ahead'
                              ? t('保留目标内容，本次不同步', 'Keep the newer target history')
                              : g.verdict === 'identical'
                                ? t('无需同步', 'No sync needed')
                                : t('暂时无法确认，不会同步', 'Cannot verify; sync disabled')
                    }}<template v-if="g.archiveAction && syncMode(g) !== 'statusOnly'">
                      · {{ t('同时同步归档', 'Also sync archive status') }}</template
                    >
                  </p>
                  <p v-if="g.verdict === 'diverge'" class="ws-warning">
                    {{
                      t(
                        `目标独有 ${g.extraB ?? 0} 条记录将被替换，无法通过本工具撤销。`,
                        `${g.extraB ?? 0} target-only records will be replaced; this tool cannot undo it.`
                      )
                    }}
                  </p>
                </div>
                <span class="ws-status" :class="g.verdict">{{ verdict(g) }}</span>
                <button
                  class="ws-expand"
                  :aria-expanded="detail.has(g.groupId)"
                  :aria-label="t('会话详情', 'Session details')"
                  @click="toggle(detail, g.groupId)"
                >
                  {{ detail.has(g.groupId) ? '−' : '+' }}
                </button>
              </div>
              <div v-if="detail.has(g.groupId)" class="ws-detail">
                <p>{{ g.reason }}</p>
                <p v-if="g.cwd">{{ g.cwd }}</p>
                <p>
                  {{ t('内容记录数（非对话轮数）', 'Content records (not conversation turns)') }} ·
                  {{ t('当前', 'Current') }} {{ g.recordCount?.source ?? '—' }} /
                  {{ t('目标', 'Target') }} {{ g.recordCount?.target ?? '—' }} /
                  {{ t('基线', 'Baseline') }} {{ g.recordCount?.baseline ?? '—' }}
                </p>
              </div>
            </div>
          </div>
        </template>
        <template v-else>
          <div class="ws-info">
            <IconCopy />
            <div>
              <b>{{ t('把对话带到新账号', 'Take conversations with you') }}</b>
              <p>
                {{
                  target === 'workbuddy'
                    ? t(
                        '复制正文和会话记录，并与原会话建立关联。目标已有副本时会跳过重复复制，后续可在关联会话中同步。',
                        'Copies history and session records, linking them to the originals. Existing linked copies are reused for future syncing.'
                      )
                    : t(
                        '复制正文和会话记录，并与原会话建立关联。重复复制可能创建新副本；更新已有副本请使用「关联会话」。',
                        'Copies history and session records, linking them to the originals. Repeating a copy may create another copy; use Linked sessions to update existing copies.'
                      )
                }}
              </p>
            </div>
          </div>
          <div class="ws-toolbar">
            <a-checkbox
              :model-value="allCopy"
              :disabled="working || !copyable.length"
              @change="toggleCopies"
              >{{ t('全选当前结果', 'Select filtered results') }}</a-checkbox
            ><a-input-search
              v-model="query"
              allow-clear
              :placeholder="t('搜索会话或项目', 'Search sessions or projects')"
              style="width: 230px"
            />
          </div>
          <div class="ws-list" role="tabpanel">
            <a-empty
              v-if="!filtered.length"
              :description="t('没有可复制的会话', 'No sessions to copy')"
            />
            <div
              v-for="s in filtered"
              :key="sessionKey(s)"
              class="ws-session"
              :class="{ picked: copied.has(sessionKey(s)) }"
            >
              <div class="ws-session-row">
                <a-checkbox
                  :model-value="copied.has(sessionKey(s))"
                  :disabled="working || !canCopySession(preview, s)"
                  :aria-label="s.title"
                  @change="toggle(copied, sessionKey(s))"
                />
                <div class="ws-session-main">
                  <strong>{{ s.title || t('无标题', 'Untitled') }}</strong>
                  <p>
                    {{
                      s.cwd ||
                      (s.isPlayground ? t('任务', 'Task') : s.workspaceHash || t('空间', 'Space'))
                    }}
                  </p>
                </div>
                <small>{{
                  s.hasHistory === false
                    ? t('无本地正文', 'No local history')
                    : fmtShort(s.updatedAt)
                }}</small>
              </div>
            </div>
          </div>
          <p v-if="preview.skipped" class="ws-notice">
            {{
              t(
                `已略过 ${preview.skipped} 个无法读取的索引`,
                `${preview.skipped} unreadable indexes skipped`
              )
            }}
          </p>
        </template>
      </template>
    </div>
    <div class="ws-footer">
      <div aria-live="polite">
        <strong>{{
          working
            ? progress
            : result
              ? t('切换完成', 'Switch complete')
              : t(
                  `同步 ${selected.length} 个 · 复制 ${copied.size} 个`,
                  `Sync ${selected.length} · Copy ${copied.size}`
                )
        }}</strong
        ><small v-if="!working && !result">{{
          t('未选择的会话保留原状', 'Unselected sessions stay as they are')
        }}</small>
      </div>
      <a-button :disabled="working" @click="close">{{
        result ? t('完成', 'Done') : t('取消', 'Cancel')
      }}</a-button>
      <a-button
        v-if="!result"
        type="primary"
        :loading="working"
        :disabled="loading"
        @click="confirm"
        >{{ t('确认切换', 'Confirm switch') }}</a-button
      >
    </div>
  </a-modal>
</template>

<style>
.wb-switch-modal {
  max-width: calc(100vw - 32px);
  border-radius: 18px;
}
.wb-switch-modal .arco-modal-body {
  padding: 4px 24px 20px;
  max-height: calc(100vh - 140px);
  overflow: hidden;
  display: flex;
  flex-direction: column;
}
.ws-content {
  min-height: 0;
  overflow-y: auto;
}
.ws-title {
  display: inline-flex;
  align-items: center;
  gap: 9px;
  font-weight: 650;
}
.ws-title svg {
  color: var(--accent);
}
.ws-route {
  display: flex;
  align-items: center;
  gap: 12px;
  padding: 16px;
  background: var(--surface-2, var(--color-fill-1));
  border: 1px solid var(--line);
  border-radius: 12px;
}
.ws-client-icon {
  width: 44px;
  height: 44px;
  border-radius: 12px;
  background: var(--accent);
  color: white;
  display: grid;
  place-items: center;
  font: 700 21px monospace;
}
.ws-route strong,
.ws-route small {
  display: block;
}
.ws-route strong {
  margin-top: 4px;
  font-size: 17px;
  overflow-wrap: anywhere;
}
.ws-route small,
.ws-route-label,
.ws-notice {
  color: var(--muted);
  font-size: 12px;
  line-height: 1.7;
}
.ws-route-label {
  margin-left: auto;
  white-space: nowrap;
}
.ws-notice {
  margin: 12px 0 16px;
}
.ws-alert {
  margin: 12px 0;
  overflow-wrap: anywhere;
}
.ws-tabs {
  display: flex;
  gap: 22px;
  align-items: center;
  border-bottom: 1px solid var(--line);
  margin-bottom: 16px;
}
.ws-tabs > button:not(.arco-btn) {
  display: flex;
  align-items: center;
  gap: 7px;
  background: transparent;
  color: var(--muted);
  padding: 12px 0;
  border: 0;
  border-bottom: 2px solid transparent;
  font: inherit;
  cursor: pointer;
}
.ws-tabs > button.active {
  border-bottom-color: var(--accent);
  color: var(--text);
  font-weight: 600;
}
.ws-tabs > button span {
  font-size: 11px;
  border-radius: 6px;
  padding: 1px 6px;
  background: var(--color-fill-2);
}
.ws-tabs .arco-btn {
  margin-left: auto;
}
.ws-info {
  display: flex;
  gap: 12px;
  padding: 14px;
  background: var(--color-fill-1);
  border-radius: 10px;
}
.ws-info > svg {
  font-size: 20px;
  color: var(--accent);
  flex: none;
  margin-top: 2px;
}
.ws-info b {
  font-size: 13px;
}
.ws-info p {
  color: var(--muted);
  font-size: 12px;
  line-height: 1.7;
  margin: 5px 0 0;
}
.ws-toolbar {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
  margin: 16px 0 10px;
}
.ws-toolbar small,
.ws-session-row > small {
  font-size: 11px;
  color: var(--muted);
}
.ws-list {
  min-height: 160px;
  max-height: 320px;
  overflow-y: auto;
  border: 1px solid var(--line);
  border-radius: 12px;
}
.ws-session {
  padding: 15px;
  border-bottom: 1px solid var(--line);
}
.ws-session:last-child {
  border-bottom: 0;
}
.ws-session.picked {
  background: color-mix(in srgb, var(--accent) 5%, transparent);
}
.ws-session-row {
  display: flex;
  align-items: center;
  gap: 12px;
}
.ws-session-main {
  flex: 1;
  min-width: 0;
}
.ws-session-main strong {
  font-size: 13px;
  line-height: 1.5;
  overflow-wrap: anywhere;
}
.ws-session-main p {
  margin: 5px 0 0;
  font-size: 12px;
  color: var(--muted);
  line-height: 1.6;
  overflow-wrap: anywhere;
}
.ws-session-main .ws-warning {
  color: var(--warn);
}
.ws-status {
  flex: none;
  border-radius: 6px;
  background: var(--color-fill-2);
  color: var(--muted);
  padding: 4px 7px;
  font-size: 11px;
}
.ws-status.fastForward {
  color: var(--accent);
  background: color-mix(in srgb, var(--accent) 9%, transparent);
}
.ws-status.diverge,
.ws-status.ahead {
  color: #b86c19;
  background: #e4a44215;
}
.ws-expand {
  border: 1px solid var(--line);
  border-radius: 6px;
  background: transparent;
  color: var(--muted);
  width: 24px;
  height: 24px;
  cursor: pointer;
}
.ws-detail {
  margin: 12px 0 0 28px;
  padding: 8px 12px;
  border-left: 2px solid var(--line);
  font-size: 12px;
  line-height: 1.6;
  color: var(--muted);
  overflow-wrap: anywhere;
}
.ws-empty {
  min-height: 260px;
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  color: var(--muted);
}
.ws-footer {
  flex: none;
  display: flex;
  align-items: center;
  gap: 10px;
  padding-top: 20px;
  margin-top: 16px;
  border-top: 1px solid var(--line);
}
.ws-footer > div {
  flex: 1;
  min-width: 0;
}
.ws-footer strong {
  font-size: 13px;
  line-height: 1.6;
  display: block;
}
.ws-footer small {
  display: block;
  margin-top: 4px;
  color: var(--muted);
  font-size: 11px;
}
.ws-result {
  padding: 25px 0;
  text-align: center;
}
.ws-result > svg {
  font-size: 36px;
  color: var(--accent);
}
.ws-result p,
.ws-path {
  font-size: 12px;
  color: var(--muted);
  overflow-wrap: anywhere;
}
@media (max-width: 600px) {
  .ws-status {
    max-width: 76px;
    white-space: normal;
  }
  .ws-route-label {
    display: none;
  }
  .ws-toolbar {
    flex-wrap: wrap;
  }
  .ws-footer {
    flex-wrap: wrap;
  }
  .ws-footer > div {
    flex-basis: 100%;
  }
}
</style>
