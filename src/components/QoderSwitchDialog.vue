<script setup lang="ts">
// Qoder account switch with session tools: sync linked groups or copy
// sessions first (copies are linked automatically), then switch.
import { ref, watch, onMounted } from 'vue'
import { Message } from '@arco-design/web-vue'
import { call } from '../api'
import { t, fmtShort } from '../i18n'
import { refreshPool } from '../store'
const props = defineProps<{ account: any }>()
const emit = defineEmits<{ close: [] }>()
const tab = ref('links')
const groups = ref<any[] | null>(null)
const sessions = ref<any[] | null>(null)
const picked = ref<string[]>([])
const running = ref(false)
const steps = ref<string[]>([])
async function loadLinks() {
  const links = await call('qoder_session_links')
  groups.value = (links.groups || []).filter((g: any) => (g.members || []).some((m: any) => m.edition === props.account.edition))
}
async function loadSessions() {
  const r = await call('qoder_list_sessions', { edition: props.account.edition, limit: 30 })
  sessions.value = (r.sessions || []).slice(0, 12)
}
watch(tab, (v) => (v === 'links' ? loadLinks() : loadSessions()).catch((e) => Message.error(String(e))))
onMounted(() => loadLinks().catch((e) => Message.error(String(e))))
async function sync(id: string) {
  try {
    const r = await call('qoder_sync_session_group', { groupId: id })
    Message[r.ok === false ? 'error' : 'success'](r.message || t('已同步', 'Synced'))
    await loadLinks()
  } catch (e) {
    Message.error(String(e))
  }
}
function toggleAll(v: boolean) {
  picked.value = v ? (sessions.value || []).map((s) => `${s.edition}:${s.id}`) : []
}
async function confirm() {
  running.value = true
  steps.value = []
  try {
    for (const k of picked.value) {
      const i = k.indexOf(':')
      try {
        const r = await call('qoder_copy_session', { edition: k.slice(0, i), sessionId: k.slice(i + 1) })
        steps.value.push(r.message || t('已复制会话副本', 'Session copy created'))
      } catch (e) {
        steps.value.push(t('复制会话失败：', 'Copy failed: ') + e)
      }
    }
    steps.value.push(t('正在切换账号…', 'Switching account…'))
    const r = await call('qoder_switch_account', { id: props.account.id })
    steps.value.push(...(r.steps || []), r.message)
    Message[r.ok ? 'success' : 'error'](r.message)
    await refreshPool()
    if (r.ok) emit('close')
  } catch (e) {
    Message.error(String(e))
  } finally {
    running.value = false
  }
}
</script>
<template>
  <a-modal :visible="true" :title="t('切换到「', 'Switch to “') + (account.note || account.name || account.uid) + t('」', '”')" :width="620" :mask-closable="!running" @cancel="emit('close')">
    <p class="hint" style="margin-top: 0">{{ t('将退出 Qoder，写入目标账号登录态（切换前自动备份）后重新打开。', 'Qoder quits, the target login is written (backed up first) and Qoder reopens.') }}</p>
    <a-tabs v-model:active-key="tab">
      <a-tab-pane key="links" :title="t('关联会话', 'Linked sessions')">
        <div v-if="groups && !groups.length" class="empty-state">
          {{ t('该版本暂无关联会话组', 'No linked session groups') }}<br /><small>{{ t('复制会话副本后会自动建立关联，切换时可一键同步', 'Copying a session links it; sync before switching') }}</small>
        </div>
        <div v-for="g in groups || []" :key="g.id" class="box" style="padding: 12px 14px; margin-bottom: 8px">
          <div class="box-head" style="margin-bottom: 6px">
            <span class="state-pill" :class="{ warn: g.contentState !== 'consistent' }">{{ g.contentState === 'consistent' ? t('内容一致', 'In sync') : t('待同步', 'Out of sync') }}</span>
            <span class="muted">{{ (g.members || []).length }} {{ t('个副本', 'copies') }}</span>
            <a-button size="mini" @click="sync(g.id)">{{ t('同步', 'Sync') }}</a-button>
          </div>
          <div v-for="m in g.membersDetail || []" :key="m.sessionId" style="font-size: 12px">
            <b>[{{ m.edition === 'cn' ? t('国内', 'CN') : t('国际', 'Intl') }}]</b> {{ m.title || m.sessionId.slice(0, 8) }} {{ m.exists ? '' : t('（文件缺失）', '(missing)') }}
          </div>
        </div>
      </a-tab-pane>
      <a-tab-pane key="copy" :title="t('复制会话', 'Copy sessions')">
        <div v-if="sessions && !sessions.length" class="empty-state">{{ t('该版本暂无会话', 'No sessions') }}</div>
        <template v-else-if="sessions">
          <a-checkbox :model-value="picked.length === sessions.length && sessions.length > 0" @change="(v) => toggleAll(!!v)">{{ t('全选', 'Select all') }} · {{ sessions.length }}</a-checkbox>
          <a-checkbox-group v-model="picked" direction="vertical" style="margin-top: 8px; width: 100%">
            <a-checkbox v-for="s in sessions" :key="s.id" :value="`${s.edition}:${s.id}`">
              {{ s.title || t('(无标题)', '(untitled)') }} <small class="muted">{{ s.cwd }} · {{ fmtShort(s.updatedAt) }}</small>
            </a-checkbox>
          </a-checkbox-group>
        </template>
      </a-tab-pane>
    </a-tabs>
    <div v-if="steps.length" class="code-block" style="margin-top: 10px">{{ steps.join('\n') }}</div>
    <template #footer>
      <a-button :disabled="running" @click="emit('close')">{{ t('取消', 'Cancel') }}</a-button>
      <a-button type="primary" :loading="running" @click="confirm">{{ t('确认切换', 'Switch') }}{{ picked.length ? t(`（复制 ${picked.length} 个会话）`, ` (copy ${picked.length})`) : '' }}</a-button>
    </template>
  </a-modal>
</template>
