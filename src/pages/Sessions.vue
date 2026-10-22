<script setup lang="ts">
import { computed, ref, watch, onMounted } from 'vue'
import { Message, Modal } from '@arco-design/web-vue'
import { IconRefresh, IconCopy, IconDelete, IconSync } from '@arco-design/web-vue/es/icon'
import PageHead from '../components/PageHead.vue'
import ProviderMark from '../components/ProviderMark.vue'
import WorkbuddySessions from '../components/WorkbuddySessions.vue'
import { call } from '../api'
import { t, fmtShort } from '../i18n'

const provider = ref<'qoder' | 'workbuddy'>('qoder')
const wbRef = ref<InstanceType<typeof WorkbuddySessions> | null>(null)
function refresh() {
  if (provider.value === 'workbuddy') wbRef.value?.load()
  else load()
}
const edition = ref<'cn' | 'intl'>('cn')
const query = ref('')
const filter = ref<'all' | 'linked' | 'consistent' | 'modified'>('all')
const data = ref<any>(null)
const links = ref<any>({ groups: [] })
const loading = ref(false)
const working = ref('')
async function load() {
  loading.value = true
  try {
    const [r, l] = await Promise.all([call('qoder_list_sessions', { edition: edition.value, query: query.value || null, limit: 100 }), call('qoder_session_links')])
    data.value = r
    links.value = l
  } catch (e) {
    data.value = { sessions: [], error: String(e) }
  } finally {
    loading.value = false
  }
}
onMounted(load)
watch(edition, load)
const groups = computed(() => (links.value.groups || []).filter((g: any) => (g.members || []).some((m: any) => m.edition === edition.value)))
const sessions = computed(() => {
  const all = data.value?.sessions ?? []
  if (filter.value === 'linked') return all.filter((s: any) => s.groupId)
  if (filter.value === 'all') return all
  return all.filter((s: any) => s.contentState === filter.value)
})
const count = (k: string) => {
  const all = data.value?.sessions ?? []
  return k === 'all' ? all.length : k === 'linked' ? all.filter((s: any) => s.groupId).length : all.filter((s: any) => s.contentState === k).length
}
const bytes = (n: number) => (n > 1048576 ? (n / 1048576).toFixed(1) + ' MB' : n > 1024 ? (n / 1024).toFixed(1) + ' KB' : (n || 0) + ' B')
async function run(key: string, work: () => Promise<any>, ok?: string) {
  working.value = key
  try {
    const r = await work()
    Message[r?.ok === false ? 'error' : 'success'](r?.message || ok || t('完成', 'Done'))
  } catch (e) {
    Message.error(String(e))
  } finally {
    working.value = ''
    load()
  }
}
const sync = (id: string) => run('sync:' + id, () => call('qoder_sync_session_group', { groupId: id }))
const copyOne = (s: any) => run('copy:' + s.id, () => call('qoder_copy_session', { edition: s.edition, sessionId: s.id }))
function remove(s: any) {
  Modal.confirm({
    title: t('删除会话', 'Delete session'),
    content: t(`删除会话「${s.title || s.id.slice(0, 8)}」？会先备份到数据目录的 session-backups，如应用正在运行将自动退出对应 Qoder。`, `Delete “${s.title || s.id.slice(0, 8)}”? It is backed up to session-backups first; a running Qoder is closed.`),
    okButtonProps: { status: 'danger' },
    onOk: () => run('del:' + s.id, () => call('qoder_delete_session', { edition: s.edition, sessionId: s.id }), t('会话已删除（已备份）', 'Session deleted (backed up)')),
  })
}
</script>

<template>
  <PageHead eyebrow="SESSIONS" :title="t('会话管理', 'Sessions')" :desc="t('管理各客户端的本地会话：复制到其他账号、关联同步与删除。写操作会先备份，必要时自动退出对应客户端。', 'Manage local client sessions: copy to other accounts, link and sync, delete. Writes back up first and quit the client when needed.')">
    <a-button :loading="loading" @click="refresh"><template #icon><IconRefresh /></template>{{ t('刷新', 'Refresh') }}</a-button>
  </PageHead>
  <a-radio-group v-model="provider" type="button" style="margin-bottom: 16px">
    <a-radio value="qoder"><ProviderMark provider="qoder" style="margin-right: 6px" />Qoder</a-radio>
    <a-radio value="workbuddy"><ProviderMark provider="workbuddy" style="margin-right: 6px" />WorkBuddy</a-radio>
  </a-radio-group>
  <WorkbuddySessions v-if="provider === 'workbuddy'" ref="wbRef" />
  <template v-else>
  <div class="filter-bar">
    <a-radio-group v-model="edition" type="button">
      <a-radio value="cn">Qoder CN</a-radio>
      <a-radio value="intl">Qoder</a-radio>
    </a-radio-group>
    <a-input-search v-model="query" :placeholder="t('搜索标题或项目路径…', 'Search title or project path…')" style="width: 300px" allow-clear @search="load" @press-enter="load" @clear="load" />
  </div>
  <section class="box">
    <div class="box-head"><h2>{{ t('关联会话', 'Linked groups') }} <span class="rtag">{{ groups.length }}</span></h2></div>
    <div v-if="!groups.length" class="muted">{{ t('暂无关联组 —— 复制会话副本后自动建立关联', 'No linked groups yet — copying a session creates one') }}</div>
    <div v-for="g in groups" :key="g.id" class="kv" style="align-items: flex-start">
      <span>
        <span class="state-pill" :class="{ warn: g.contentState !== 'consistent' }">{{ g.contentState === 'consistent' ? t('内容一致', 'In sync') : t('待同步', 'Needs sync') }}</span>
        {{ (g.members || []).length }} {{ t('个副本', 'copies') }}
      </span>
      <b style="display: flex; flex-direction: column; gap: 2px; align-items: flex-end">
        <span v-for="m in g.membersDetail || []" :key="m.sessionId" style="font-size: 12px">[{{ m.edition === 'cn' ? t('国内', 'CN') : t('国际', 'Intl') }}] {{ m.title || m.sessionId.slice(0, 8) }}{{ m.exists ? '' : t('（文件缺失）', ' (missing)') }}</span>
        <a-button size="mini" type="primary" :loading="working === 'sync:' + g.id" @click="sync(g.id)"><template #icon><IconSync /></template>{{ t('同步', 'Sync') }}</a-button>
      </b>
    </div>
  </section>
  <div class="filter-bar">
    <a-radio-group v-model="filter" type="button" size="small">
      <a-radio value="all">{{ t('全部', 'All') }} {{ count('all') }}</a-radio>
      <a-radio value="linked">{{ t('已关联', 'Linked') }} {{ count('linked') }}</a-radio>
      <a-radio value="consistent">{{ t('内容一致', 'In sync') }} {{ count('consistent') }}</a-radio>
      <a-radio value="modified">{{ t('待同步', 'Needs sync') }} {{ count('modified') }}</a-radio>
    </a-radio-group>
    <span class="muted">{{ data?.total ?? 0 }} {{ t('个会话', 'sessions') }}</span>
  </div>
  <a-alert v-if="data?.error" type="error" style="margin-bottom: 12px">{{ data.error }}</a-alert>
  <div v-if="!sessions.length && !loading" class="empty-state">{{ t('没有符合条件的会话', 'No sessions match') }}</div>
  <div class="session-grid">
    <div v-for="s in sessions" :key="s.id" class="session-card">
      <small class="mono">{{ (s.cwd || '').split('/').slice(-1).join('/') }}</small>
      <strong :title="s.title">{{ s.title || t('(无标题)', '(Untitled)') }}</strong>
      <small>{{ fmtShort(s.updatedAt) }} · {{ bytes(s.size) }}<span v-if="s.groupId"> · {{ s.contentState === 'consistent' ? t('内容一致', 'In sync') : t('待同步', 'Needs sync') }}</span></small>
      <div style="display: flex; gap: 6px">
        <a-button size="mini" :loading="working === 'copy:' + s.id" @click="copyOne(s)"><template #icon><IconCopy /></template>{{ t('复制副本', 'Copy') }}</a-button>
        <a-button v-if="s.groupId" size="mini" :loading="working === 'sync:' + s.groupId" @click="sync(s.groupId)">{{ t('同步', 'Sync') }}</a-button>
        <a-button size="mini" status="danger" :loading="working === 'del:' + s.id" @click="remove(s)"><template #icon><IconDelete /></template>{{ t('删除', 'Delete') }}</a-button>
      </div>
    </div>
  </div>
  </template>
</template>
