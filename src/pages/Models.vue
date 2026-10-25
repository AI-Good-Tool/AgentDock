<script setup lang="ts">
import ProviderMark from '../components/ProviderMark.vue'
import { computed, ref, onMounted } from 'vue'
import { Message, Modal } from '@arco-design/web-vue'
import {
  IconRefresh,
  IconPlus,
  IconCopy,
  IconEdit,
  IconLink,
  IconClose,
} from '@arco-design/web-vue/es/icon'
import PageHead from '../components/PageHead.vue'
import { call, type RouteView, type Target } from '../api'
import { t, fmtNumber, fmtShort } from '../i18n'
import {
  state,
  act,
  busy,
  refreshCatalog,
  copy,
  providerLabel,
  regionLabel,
  PROVIDERS,
} from '../store'

const provider = ref('all')
const region = ref('all')
const cap = ref('all')
const query = ref('')
const editing = ref<null | {
  route: RouteView | null
  alias: string
  mode: 'auto' | 'explicit'
  targets: string[]
  disabled: boolean
}>(null)
const pickQuery = ref('')
onMounted(refreshCatalog)

const routes = computed(() =>
  (state.catalog?.routes ?? []).filter((r) => {
    const q = query.value.trim().toLowerCase()
    const text =
      `${r.alias} ${r.defaultAlias} ${r.allTargets.map((x) => `${x.id} ${x.name}`).join(' ')}`.toLowerCase()
    return (
      (!q || text.includes(q)) &&
      (provider.value === 'all' || r.allTargets.some((x) => x.provider === provider.value)) &&
      (region.value === 'all' || r.allTargets.some((x) => x.region === region.value)) &&
      (cap.value === 'all' ||
        (cap.value === 'shared' &&
          new Set(r.allTargets.map((x) => `${x.provider}:${x.region}`)).size > 1) ||
        (cap.value === 'tools' && r.supportsTools) ||
        (cap.value === 'images' && r.supportsImages) ||
        (cap.value === 'custom' && (r.configured || r.linked.length > 0)) ||
        (cap.value === 'disabled' && (r.disabled || !r.available)))
    )
  })
)
const status = computed(() => state.catalog?.status ?? {})
const verified = computed(
  () => new Set(state.logs.filter((l) => l.outcome === 'completed').map((l) => l.routeId))
)

function sync(p?: string) {
  return act(
    `sync:${p ?? 'all'}`,
    async () => {
      const r = await call('sync_catalog', { providerKey: p ?? null, region: null })
      if (r.warnings?.length) Message.warning(r.warnings.join('；'))
      return r
    },
    t('模型目录已同步', 'Catalog synced'),
    [refreshCatalog]
  )
}
function open(r: RouteView | null) {
  pickQuery.value = ''
  editing.value = {
    route: r,
    alias: r?.alias ?? '',
    mode: r && !r.custom && !state.app?.settings.routes[r.routeId]?.targets ? 'auto' : 'explicit',
    targets: r ? r.allTargets.map((x) => x.key) : [],
    disabled: r?.disabled ?? false,
  }
}
const candidates = computed(() => {
  const q = pickQuery.value.trim().toLowerCase()
  return (state.catalog?.upstreams ?? []).filter(
    (u) => !q || `${u.id} ${u.name} ${u.provider} ${u.region}`.toLowerCase().includes(q)
  )
})
async function save() {
  const e = editing.value!
  const targets = e.mode === 'explicit' ? e.targets : null
  if (e.mode === 'explicit' && !e.targets.length)
    return Message.error(t('请至少选择一个上游模型', 'Select at least one upstream model'))
  const ok = await act(
    'route',
    () =>
      e.route
        ? call('save_route', {
            routeId: e.route.routeId,
            alias: e.alias.trim() === e.route.defaultAlias ? '' : e.alias.trim(),
            targets,
            disabled: e.disabled,
          })
        : call('create_route', { alias: e.alias, targets: e.targets }),
    t('模型映射已保存，对新请求立即生效', 'Mapping saved; applies to new requests'),
    [refreshCatalog]
  )
  if (ok !== undefined) editing.value = null
}
function reset(r: RouteView) {
  Modal.confirm({
    title: t('恢复自动映射', 'Restore automatic mapping'),
    content: t(
      '删除此模型的别名和手动上游设置，恢复按官方名称自动合并。',
      'Remove the alias and manual upstreams and merge by official name again.'
    ),
    onOk: () =>
      act('route', () => call('reset_route', { routeId: r.routeId }), t('已恢复', 'Restored'), [
        refreshCatalog,
      ]),
  })
}
// Merge other models into one: their accounts join this model's scheduling.
const linking = ref<null | { route: RouteView; selected: string[]; query: string }>(null)
function openLink(r: RouteView) {
  linking.value = { route: r, selected: r.linked.map((l) => l.routeId), query: '' }
}
const linkCandidates = computed(() => {
  const l = linking.value
  if (!l) return []
  const q = l.query.trim().toLowerCase()
  const merged = l.route.linked.map((x) => ({
    routeId: x.routeId,
    alias: x.alias,
    targets: l.route.allTargets.filter((t) => x.targets.includes(t.key)),
    blocked: '',
  }))
  const others = (state.catalog?.routes ?? [])
    .filter((r) => r.routeId !== l.route.routeId)
    .map((r) => ({
      routeId: r.routeId,
      alias: r.alias,
      targets: r.allTargets,
      blocked: r.linked.length
        ? t('已有其他模型关联到它，不能再并入别的模型', 'Other models are linked into it')
        : r.disabled
          ? t('已停用的模型不能关联，请先启用', 'Enable this model before linking it')
          : '',
    }))
  return [...merged, ...others].filter(
    (c) =>
      !q ||
      `${c.alias} ${c.targets.map((x) => `${x.id} ${x.name}`).join(' ')}`.toLowerCase().includes(q)
  )
})
async function saveLinks() {
  const l = linking.value!
  const before = l.route.linked.map((x) => x.routeId)
  const ok = await act(
    'link',
    async () => {
      try {
        for (const id of before.filter((x) => !l.selected.includes(x)))
          await call('link_route', { routeId: id, into: null })
        for (const id of l.selected.filter((x) => !before.includes(x)))
          await call('link_route', { routeId: id, into: l.route.routeId })
      } catch (e) {
        await refreshCatalog()
        throw e
      }
      return { ok: true }
    },
    t('关联已保存，对新请求立即生效', 'Links saved; applies to new requests'),
    [refreshCatalog]
  )
  if (ok !== undefined) linking.value = null
}
function unlink(routeId: string) {
  return act(
    'link',
    () => call('link_route', { routeId, into: null }),
    t('已取消关联', 'Unlinked'),
    [refreshCatalog]
  )
}
const linkedFrom = (x: Target, r: RouteView) =>
  r.linked.find((l) => l.targets.includes(x.key))?.alias
const tags = (r: RouteView) =>
  [
    ...r.efforts.map((x) =>
      x === 'none' ? t('可关闭思考', 'Thinking off') : `${t('思考', 'Effort')} ${x}`
    ),
    r.supportsTools ? t('工具调用', 'Tools') : null,
    r.supportsImages ? t('图片', 'Images') : null,
    r.contextWindow ? `${fmtNumber(r.contextWindow, 0)} context` : null,
  ].filter(Boolean) as string[]
const inScope = (x: Target, r: RouteView) => r.targets.some((y) => y.key === x.key)
const regionCount = (targets: Target[]) => new Set(targets.map((x) => x.provider + x.region)).size
</script>

<template>
  <PageHead
    eyebrow="MODEL CATALOG"
    :title="t('模型目录', 'Models')"
    :desc="
      t(
        '官方名称一致的模型跨端、跨地区自动合并为一个对外模型；可编辑别名与参与调度的上游。',
        'Models with the same official name merge across providers and regions into one public model. Edit aliases and upstreams.'
      )
    "
  >
    <a-dropdown trigger="click">
      <a-button :loading="busy('sync:workbuddy') || busy('sync:qoder') || busy('sync:trae')"
        ><template #icon><IconRefresh /></template
        >{{ t('同步单端目录', 'Sync one provider') }}</a-button
      >
      <template #content>
        <a-doption v-for="p in PROVIDERS" :key="p.key" @click="sync(p.key)">{{
          p.label
        }}</a-doption>
      </template>
    </a-dropdown>
    <a-button :loading="busy('sync:all')" @click="sync()"
      ><template #icon><IconRefresh /></template
      >{{ t('同步全部目录', 'Sync all catalogs') }}</a-button
    >
    <a-button type="primary" @click="open(null)"
      ><template #icon><IconPlus /></template>{{ t('新建映射', 'New mapping') }}</a-button
    >
  </PageHead>

  <div class="metric-row">
    <div v-for="p in PROVIDERS" :key="p.key" class="metric">
      <span><ProviderMark :provider="p.key" style="margin-right: 6px" />{{ p.label }}</span>
      <strong
        >{{ (status[p.key]?.cn?.count ?? 0) + (status[p.key]?.intl?.count ?? 0)
        }}<small>{{ t('个上游模型', 'upstream models') }}</small></strong
      >
      <p>
        {{ t('国内', 'CN') }} {{ status[p.key]?.cn?.count ?? 0 }} · {{ t('国际', 'Intl') }}
        {{ status[p.key]?.intl?.count ?? 0
        }}<template v-if="p.key === 'qoder' && status.qoder?.cn?.updated">
          · {{ fmtShort(status.qoder.cn.updated) }}</template
        ><template v-if="p.key === 'workbuddy' && status.workbuddy?.cn?.bundled">
          · {{ t('内置快照', 'bundled snapshot') }}</template
        >
      </p>
    </div>
    <div class="metric">
      <span>{{ t('对外模型', 'Public models') }}</span>
      <strong
        >{{ state.catalog?.routes.filter((r) => r.available).length ?? 0
        }}<small>/ {{ state.catalog?.routes.length ?? 0 }}</small></strong
      >
      <p>{{ t('可用 / 全部（含停用与不在范围内）', 'available / all') }}</p>
    </div>
  </div>

  <div class="filter-bar">
    <a-select v-model="provider" style="width: 150px">
      <a-option value="all">{{ t('全部端', 'All providers') }}</a-option>
      <a-option v-for="p in PROVIDERS" :key="p.key" :value="p.key">{{ p.label }}</a-option>
    </a-select>
    <a-select v-model="region" style="width: 130px">
      <a-option value="all">{{ t('全部地区', 'All regions') }}</a-option>
      <a-option value="cn">{{ t('国内版', 'China') }}</a-option>
      <a-option value="intl">{{ t('国际版', 'International') }}</a-option>
    </a-select>
    <a-radio-group v-model="cap" type="button">
      <a-radio value="all">{{ t('全部', 'All') }}</a-radio>
      <a-radio value="shared">{{ t('多端共有', 'Shared') }}</a-radio>
      <a-radio value="tools">{{ t('工具调用', 'Tools') }}</a-radio>
      <a-radio value="images">{{ t('图片', 'Images') }}</a-radio>
      <a-radio value="custom">{{ t('已自定义', 'Customized') }}</a-radio>
      <a-radio value="disabled">{{ t('停用/不可用', 'Unavailable') }}</a-radio>
    </a-radio-group>
    <span class="grow"></span>
    <a-input-search
      v-model="query"
      :placeholder="t('搜索别名、名称或模型 ID', 'Search alias, name or ID')"
      style="width: 260px"
      allow-clear
    />
  </div>

  <div v-if="!routes.length" class="empty-state">
    <strong>{{
      state.catalog?.routes.length
        ? t('没有匹配的模型', 'No matching models')
        : t('先同步你的模型目录', 'Sync your model catalogs')
    }}</strong>
    {{
      t(
        '同步只读取官方目录，不会发起聊天请求。Trae 模型来自各账号的模型列表。',
        'Syncing reads official catalogs only; no chat requests. Trae models come from each account.'
      )
    }}
  </div>
  <div class="route-grid">
    <article
      v-for="r in routes"
      :key="r.routeId"
      class="route-card"
      :class="{ disabled: r.disabled || !r.available }"
    >
      <div style="display: flex; justify-content: space-between; gap: 8px; align-items: flex-start">
        <div style="min-width: 0">
          <h3>{{ r.alias }}</h3>
          <div class="sub mono">
            {{ r.alias !== r.defaultAlias ? t('默认：', 'default: ') + r.defaultAlias : r.routeId }}
          </div>
        </div>
        <span class="state-pill" :class="{ off: !verified.has(r.routeId), bad: r.disabled }">{{
          r.disabled
            ? t('已停用', 'Disabled')
            : !r.available
              ? t('不在范围内', 'Out of scope')
              : verified.has(r.routeId)
                ? t('已调用', 'Called')
                : t('目录可见', 'In catalog')
        }}</span>
      </div>
      <div
        v-for="x in r.allTargets"
        :key="x.key"
        class="target-row"
        :class="{ out: !inScope(x, r) }"
      >
        <span class="ptag"
          ><ProviderMark :provider="x.provider" />{{ providerLabel(x.provider) }}</span
        >
        <span class="rtag">{{ regionLabel(x.region) }}</span>
        <code :title="x.name + ' · ' + x.id"
          >{{ x.name !== x.id ? x.name + ' · ' : '' }}{{ x.id }}</code
        >
        <span
          v-if="linkedFrom(x, r)"
          class="rtag via-tag"
          :title="t('由关联的模型「', 'From linked model “') + linkedFrom(x, r) + t('」合并而来', '”')"
          ><IconLink /></span
        >
        <span
          v-if="x.supportsTools === false"
          class="rtag text-only"
          :title="t('带工具或图片的请求（Claude Code、Codex 等）不会调度到这里', 'Requests with tools or images (Claude Code, Codex…) are not routed here')"
          >{{ x.supportsImages === false ? t('仅纯文本', 'Text only') : t('无工具', 'No tools') }}</span
        >
      </div>
      <div v-for="m in r.missing" :key="m" class="target-row out">
        <span class="state-pill warn">{{ t('缺失', 'Missing') }}</span
        ><span></span><code>{{ m }}</code>
      </div>
      <div v-if="r.linked.length" class="linked-row">
        <span class="linked-label"><IconLink />{{ t('已关联', 'Linked') }}</span>
        <span v-for="l in r.linked" :key="l.routeId" class="linked-chip" :title="l.alias"
          ><span class="linked-name">{{ l.alias }}</span
          ><button
            type="button"
            class="linked-x"
            :disabled="busy('link')"
            :title="t('取消关联', 'Unlink')"
            :aria-label="t('取消关联 ', 'Unlink ') + l.alias"
            @click="unlink(l.routeId)"
          >
            <IconClose /></button
        ></span>
      </div>
      <div class="cap-tags">
        <span v-for="tag in tags(r)" :key="tag">{{ tag }}</span>
        <span v-if="!tags(r).length">{{
          t('能力以实际调用为准', 'Capabilities depend on model')
        }}</span>
      </div>
      <div class="route-foot">
        <span class="route-count"
          >{{ regionCount(r.targets) }} {{ t('个端/地区参与调度', 'provider-regions scheduled')
          }}<template v-if="regionCount(r.targets.filter((x) => x.supportsTools !== false)) < regionCount(r.targets)">
            · {{ t('工具请求', 'with tools') }}
            {{ regionCount(r.targets.filter((x) => x.supportsTools !== false)) }}</template
          ></span
        >
        <div class="route-actions">
          <a-button v-if="r.configured" size="mini" @click="reset(r)">{{
            t('恢复自动', 'Reset')
          }}</a-button>
          <a-button size="mini" @click="openLink(r)"
            ><template #icon><IconLink /></template>{{ t('关联模型', 'Link models') }}</a-button
          >
          <a-button size="mini" @click="open(r)"
            ><template #icon><IconEdit /></template
            >{{ t('别名 / 路由', 'Alias / routing') }}</a-button
          >
          <a-button size="mini" @click="copy(r.alias)"
            ><template #icon><IconCopy /></template
          ></a-button>
        </div>
      </div>
    </article>
  </div>

  <a-modal
    v-if="editing"
    :visible="true"
    :width="680"
    :title="
      editing.route
        ? t('模型别名与路由', 'Alias and routing')
        : t('新建模型映射', 'New model mapping')
    "
    :ok-loading="busy('route')"
    @ok="save"
    @cancel="editing = null"
  >
    <label class="muted">{{ t('对外模型别名', 'Public model alias') }}</label>
    <a-input
      v-model="editing.alias"
      :max-length="128"
      :placeholder="editing.route?.defaultAlias"
      style="margin: 6px 0 4px"
    />
    <p class="hint" style="margin-top: 0">
      {{
        t(
          '客户端使用此别名，响应与请求记录也显示此别名；留空恢复默认名称。每次请求只选择一个符合映射的可用账号。',
          'Clients use this alias and see it in responses and logs; leave empty for the default. Each request uses exactly one eligible account.'
        )
      }}
    </p>
    <template v-if="editing.route">
      <a-radio-group v-model="editing.mode" type="button" style="margin: 10px 0">
        <a-radio value="auto">{{
          t('自动（按官方名称合并）', 'Automatic (merge by official name)')
        }}</a-radio>
        <a-radio value="explicit">{{ t('手动选择上游', 'Choose upstreams') }}</a-radio>
      </a-radio-group>
      <a-checkbox v-model="editing.disabled" style="margin-left: 12px">{{
        t('停用此模型', 'Disable this model')
      }}</a-checkbox>
    </template>
    <template v-if="editing.mode === 'explicit' || !editing.route">
      <a-input-search
        v-model="pickQuery"
        :placeholder="t('搜索上游模型', 'Search upstream models')"
        style="margin-bottom: 8px"
        allow-clear
      />
      <div class="pick-list">
        <a-checkbox-group v-model="editing.targets" direction="vertical" style="width: 100%">
          <label v-for="u in candidates" :key="u.key" class="pick-row">
            <a-checkbox :value="u.key" />
            <span class="ptag"
              ><ProviderMark :provider="u.provider" />{{ providerLabel(u.provider) }}</span
            >
            <span class="rtag">{{ regionLabel(u.region) }}</span>
            <span style="min-width: 0"
              >{{ u.name }} <code>{{ u.id }}</code></span
            >
          </label>
        </a-checkbox-group>
      </div>
      <p class="hint">
        {{
          t(
            '选中的上游只属于这个映射（会从自动合并中移出）。多个端/地区都选中时，按账号池调度策略统一轮询；只选一个时只使用该端该地区的账号。手动映射不同模型时请自行确认能力一致。',
            'Selected upstreams belong only to this mapping (removed from automatic groups). With several provider-regions selected, the scheduling strategy rotates across all of them; with one, only those accounts are used.'
          )
        }}
      </p>
    </template>
    <p v-else class="hint">
      {{ t('当前上游：', 'Current upstreams: ')
      }}{{
        editing.route?.allTargets
          .map((x) => `${providerLabel(x.provider)}/${regionLabel(x.region)}/${x.id}`)
          .join('、')
      }}
    </p>
  </a-modal>

  <a-modal
    v-if="linking"
    :visible="true"
    :width="680"
    :title="t('关联模型到 ', 'Link models into ') + linking.route.alias"
    :ok-loading="busy('link')"
    @ok="saveLinks"
    @cancel="linking = null"
  >
    <p class="hint" style="margin-top: 0">
      {{
        t(
          `勾选的模型并入「${linking.route.alias}」：它们所属的账号与此模型的账号一起按调度策略轮询；客户端用原名称或原模型 ID 请求也会走这里；模型列表只显示「${linking.route.alias}」。之后同步到的新账号、新地区会自动加入。能力按所有上游取交集（如上下文取最小值）。`,
          `Checked models merge into "${linking.route.alias}": their accounts are scheduled together with this model's; requests using their old names or IDs land here, and only "${linking.route.alias}" is listed. Newly synced accounts and regions join automatically. Capabilities are the intersection of all upstreams.`
        )
      }}
    </p>
    <a-input-search
      v-model="linking.query"
      :placeholder="t('搜索模型', 'Search models')"
      style="margin-bottom: 8px"
      allow-clear
    />
    <div class="pick-list">
      <a-checkbox-group v-model="linking.selected" direction="vertical" style="width: 100%">
        <label
          v-for="c in linkCandidates"
          :key="c.routeId"
          class="pick-row"
          style="grid-template-columns: auto auto 1fr"
          :title="c.blocked"
        >
          <a-checkbox :value="c.routeId" :disabled="!!c.blocked" />
          <strong style="white-space: nowrap">{{ c.alias }}</strong>
          <code>{{
            c.targets
              .map((x) => `${providerLabel(x.provider)}/${regionLabel(x.region)}/${x.id}`)
              .join('、')
          }}</code>
        </label>
      </a-checkbox-group>
    </div>
  </a-modal>
</template>
