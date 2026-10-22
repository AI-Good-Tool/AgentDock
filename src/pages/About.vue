<script setup lang="ts">
// About + manual update check + verified in-app update (GitHub Releases, SHA256).
import { computed, onMounted, onUnmounted, ref } from 'vue'
import { Modal, Message } from '@arco-design/web-vue'
import { listen, type UnlistenFn } from '@tauri-apps/api/event'
import { IconUser, IconCode, IconHeart, IconMessage, IconRefresh, IconDownload, IconLink, IconArrowRight } from '@arco-design/web-vue/es/icon'
import BrandMark from '../components/BrandMark.vue'
import { call, desktop, isWindows } from '../api'
import { t } from '../i18n'
import { state } from '../store'

interface UpdateInfo {
  currentVersion: string
  latestVersion: string | null
  target: string
  hasUpdate: boolean
  canDownload: boolean
  reason: string
  notes: string
  assetName: string | null
  assetSize: number | null
}
interface Download {
  version: string
  path: string
  sha256: string
  sizeBytes: number
}
interface Progress {
  status: string
  downloadedBytes: number
  totalBytes: number | null
  message: string | null
}
const version = computed(() => state.app?.version ?? '1.0.1')
const info = ref<UpdateInfo | null>(null)
const downloaded = ref<Download | null>(null)
const checking = ref(false)
const downloading = ref(false)
const installing = ref(false)
const cancelling = ref(false)
const listenerReady = ref(false)
const error = ref('')
const progress = ref<Progress | null>(null)
const busy = computed(() => checking.value || downloading.value || installing.value)
const percent = computed(() =>
  progress.value?.totalBytes ? Math.min(100, Math.floor((progress.value.downloadedBytes / progress.value.totalBytes) * 100)) : 0
)
let unlisten: UnlistenFn | undefined

const links = computed(() => [
  { id: 'author', icon: IconUser, tone: 'blue', title: t('作者主页', 'Author'), desc: t('访问 vs2pk0 的 GitHub 主页', "Visit vs2pk0's GitHub profile") },
  { id: 'repository', icon: IconCode, tone: 'gray', title: t('开源仓库', 'Repository'), desc: t('查看源码、版本和发布记录', 'Source code, versions and releases') },
  { id: 'sponsor', icon: IconHeart, tone: 'red', title: t('赞助支持', 'Sponsor'), desc: t('查看支付宝、微信和 Binance 收款码', 'Alipay, WeChat and Binance codes') },
  { id: 'issues', icon: IconMessage, tone: 'green', title: t('问题反馈', 'Feedback'), desc: t('提交 Issue 或改进建议', 'Open an issue or suggest an improvement') },
])
const mb = (n: number | null | undefined) => (n == null ? '—' : `${(n / 1024 / 1024).toFixed(1)} MB`)

async function open(destination: string) {
  if (!desktop) return Message.info(t('请在桌面应用中使用此功能', 'Available in the desktop app'))
  try {
    await call('open_about_link', { destination })
  } catch (e) {
    Message.error(String(e))
  }
}
async function check() {
  if (busy.value || !desktop) return
  checking.value = true
  error.value = ''
  try {
    info.value = await call<UpdateInfo>('app_update_check')
    if (downloaded.value?.version !== info.value.latestVersion) downloaded.value = null
    if (!info.value.hasUpdate) Message.success(info.value.reason)
  } catch (e) {
    error.value = String(e)
    info.value = null
  } finally {
    checking.value = false
  }
}
function download() {
  const latest = info.value?.latestVersion
  if (!latest || !info.value?.canDownload || busy.value || !listenerReady.value) return
  Modal.confirm({
    title: t(`下载 AgentDock ${latest}？`, `Download AgentDock ${latest}?`),
    content: t(
      `来源：AI-Good-Tool/AgentDock Releases，大小 ${mb(info.value.assetSize)}。下载后会校验 SHA256，不会自动安装，也不会中断当前请求。`,
      `From AI-Good-Tool/AgentDock Releases, ${mb(info.value.assetSize)}. The SHA256 is verified after download; nothing is installed or interrupted automatically.`
    ),
    onOk: () => void startDownload(latest),
  })
}
async function startDownload(latest: string) {
  downloading.value = true
  cancelling.value = false
  downloaded.value = null
  error.value = ''
  progress.value = null
  try {
    downloaded.value = await call<Download>('app_update_download', { expectedVersion: latest, confirmed: true })
  } catch (e) {
    if (!cancelling.value) error.value = String(e)
  } finally {
    downloading.value = false
    cancelling.value = false
  }
}
async function cancel() {
  cancelling.value = true
  try {
    await call('app_update_cancel_download')
  } catch (e) {
    error.value = String(e)
    cancelling.value = false
  }
}
function install() {
  if (!downloaded.value || busy.value) return
  Modal.confirm({
    title: t('安装更新并退出应用？', 'Install the update and quit?'),
    content: isWindows
      ? t('反代会停止（进行中的请求最多等待 15 秒），然后打开安装向导；安装完成后重新打开 AgentDock。安装包未经开发者证书签名，请按系统提示确认。', 'The gateway stops (in-flight requests get up to 15 s), then the installer opens; reopen AgentDock afterwards. The installer is not code-signed; follow the system prompts.')
      : t('反代会停止（进行中的请求最多等待 15 秒），然后替换已安装的应用并自动重启；失败会自动回滚到当前版本。', 'The gateway stops (in-flight requests get up to 15 s), then the installed app is replaced and restarted; failures roll back to this version.'),
    okText: t('确认安装并退出', 'Install and quit'),
    onOk: async () => {
      installing.value = true
      error.value = ''
      try {
        await call('app_update_install', { confirmed: true })
      } catch (e) {
        error.value = String(e)
        installing.value = false
      }
    },
  })
}
onMounted(async () => {
  if (!desktop) return
  try {
    unlisten = await listen<Progress>('agentdock-update-progress', (e) => (progress.value = e.payload))
    listenerReady.value = true
  } catch (e) {
    error.value = t('更新进度监听失败：', 'Update progress unavailable: ') + String(e)
  }
})
onUnmounted(() => unlisten?.())
</script>

<template>
  <section class="about-page">
    <div class="about-hero">
      <div class="about-icon"><BrandMark /></div>
      <div class="about-kicker">{{ t('本地多平台模型网关', 'Local multi-provider model gateway') }}</div>
      <h1>AgentDock</h1>
      <div class="about-badges">
        <span class="mono">v{{ version }}</span>
        <button @click="open('repository')"><IconLink /> GitHub</button>
      </div>
      <p>{{ t('把 WorkBuddy、Qoder、Trae 的账号、签到、会话、用量统计和本地 API 服务收在一个干净的桌面工作台里。', 'WorkBuddy, Qoder and Trae accounts, check-ins, sessions, usage statistics and a local API gateway in one clean desktop workspace.') }}</p>
    </div>

    <div class="about-grid">
      <button v-for="l in links" :key="l.id" class="about-card" @click="open(l.id)">
        <span class="about-symbol" :class="l.tone"><component :is="l.icon" /></span>
        <span class="about-text"><strong>{{ l.title }}</strong><small>{{ l.desc }}</small></span>
      </button>
      <button class="about-card" :disabled="!desktop || busy" @click="check">
        <span class="about-symbol purple"><IconRefresh :spin="checking" /></span>
        <span class="about-text"
          ><strong>{{ checking ? t('正在检查…', 'Checking…') : t('检查更新', 'Check for updates') }}</strong
          ><small>{{ t('查看最新版本并下载安装包', 'Find the latest release and its installer') }}</small></span
        >
      </button>
      <button class="about-card" :disabled="!info?.canDownload || busy || !listenerReady" @click="download">
        <span class="about-symbol green"><IconDownload /></span>
        <span class="about-text"
          ><strong>{{ downloading ? t('正在下载…', 'Downloading…') : t('在线更新', 'Update in app') }}</strong
          ><small>{{ info?.hasUpdate ? t(`下载 v${info.latestVersion}（${mb(info.assetSize)}）`, `Download v${info.latestVersion} (${mb(info.assetSize)})`) : t('检查到新版本后可直接下载并安装', 'Available once a newer version is found') }}</small></span
        >
      </button>
    </div>

    <div v-if="info || error || downloading || downloaded" class="update-panel" aria-live="polite">
      <template v-if="info">
        <h3>{{ info.hasUpdate ? t(`发现新版本 v${info.latestVersion}`, `New version v${info.latestVersion}`) : t('版本检查结果', 'Update check') }}</h3>
        <p>{{ info.reason }}</p>
        <small class="muted"
          >{{ t('当前', 'Current') }} v{{ info.currentVersion }} · {{ info.target }}<template v-if="info.assetName"> · {{ info.assetName }} · {{ mb(info.assetSize) }}</template></small
        >
        <details v-if="info.notes">
          <summary>{{ t('更新说明', 'Release notes') }}</summary>
          <pre>{{ info.notes }}</pre>
        </details>
      </template>
      <template v-if="downloading">
        <p>{{ cancelling ? t('正在取消下载…', 'Cancelling…') : t(`下载进度 ${percent}%`, `Downloading ${percent}%`) }} · {{ mb(progress?.downloadedBytes) }} / {{ mb(progress?.totalBytes) }}</p>
        <a-progress :percent="percent / 100" :show-text="false" />
        <a-button size="small" :disabled="cancelling" @click="cancel">{{ t('取消下载', 'Cancel') }}</a-button>
      </template>
      <template v-if="downloaded">
        <p>{{ t(`安装包已下载并通过 SHA256 校验 · v${downloaded.version}`, `Downloaded and SHA256-verified · v${downloaded.version}`) }}</p>
        <small class="update-hash mono">SHA256 {{ downloaded.sha256 }}</small>
        <a-button type="primary" :loading="installing" :disabled="busy && !installing" @click="install">{{ installing ? t('正在准备安装…', 'Preparing…') : t('安装更新并退出', 'Install and quit') }}</a-button>
      </template>
      <p v-if="error" class="error-text" role="alert">{{ error }}</p>
      <a-button size="small" @click="open('releases')">{{ t('前往 Releases 手动下载', 'Open Releases') }} <IconArrowRight /></a-button>
    </div>
    <p class="about-note">{{ t('只在点击检查时连接 GitHub，不发送任何账号凭据；检查和下载不会影响反代，安装前需要再次确认。', 'GitHub is contacted only when you check; no account credentials are sent. Checking and downloading never affect the gateway; installing asks again.') }}</p>
  </section>
</template>

<style scoped>
.about-page {
  max-width: 1080px;
  margin: 0 auto;
  padding: 8px 0 24px;
}
.about-hero {
  text-align: center;
  padding: 8px 12px 30px;
}
.about-icon {
  width: 104px;
  height: 104px;
  margin: 0 auto 20px;
  padding: 12px;
  border-radius: 30px;
  background: var(--surface);
  border: 1px solid var(--line);
  box-shadow: 0 18px 48px rgba(16, 154, 115, 0.12);
}
.about-icon :deep(.brand-mark) {
  width: 100%;
  height: 100%;
  border-radius: 18px;
  box-shadow: none;
}
.about-kicker {
  font-weight: 600;
  color: var(--accent);
  font-size: 14px;
}
.about-hero h1 {
  font-size: 38px;
  letter-spacing: -1.2px;
  margin: 12px 0 18px;
  color: var(--text);
}
.about-badges {
  display: inline-flex;
  align-items: center;
  gap: 6px;
  padding: 5px;
  border: 1px solid var(--line);
  background: var(--surface-2);
  border-radius: 30px;
}
.about-badges span {
  background: var(--surface);
  border: 1px solid var(--line);
  border-radius: 24px;
  padding: 7px 18px;
  font-weight: 700;
  color: var(--text);
  font-size: 14px;
}
.about-badges button {
  display: inline-flex;
  align-items: center;
  gap: 6px;
  border: 0;
  background: none;
  padding: 7px 16px;
  color: var(--text-2);
  cursor: pointer;
  font-weight: 600;
  font-size: 14px;
}
.about-hero p {
  color: var(--muted);
  line-height: 1.8;
  margin: 22px 0 0;
}
.about-grid {
  display: grid;
  grid-template-columns: repeat(2, minmax(0, 1fr));
  gap: 16px;
}
.about-card {
  display: flex;
  align-items: center;
  gap: 18px;
  text-align: left;
  padding: 22px 24px;
  border-radius: 16px;
  border: 1px solid var(--line);
  background: var(--surface);
  cursor: pointer;
  box-shadow: var(--shadow);
  color: var(--text);
  transition: transform 0.15s, border-color 0.15s;
}
.about-card:hover:not(:disabled) {
  transform: translateY(-2px);
  border-color: var(--accent);
}
.about-card:disabled {
  cursor: default;
  opacity: 0.55;
}
.about-symbol {
  display: flex;
  align-items: center;
  justify-content: center;
  flex: none;
  width: 52px;
  height: 52px;
  border-radius: 16px;
  font-size: 24px;
  background: var(--surface-2);
  color: var(--text-2);
}
.about-symbol.blue {
  background: rgba(59, 130, 246, 0.12);
  color: #3b6fd8;
}
.about-symbol.red {
  background: rgba(239, 68, 68, 0.12);
  color: #d64545;
}
.about-symbol.green {
  background: var(--accent-soft);
  color: var(--accent);
}
.about-symbol.purple {
  background: rgba(139, 92, 246, 0.13);
  color: #8458d2;
}
.about-text strong {
  display: block;
  font-size: 17px;
  margin-bottom: 6px;
}
.about-text small {
  display: block;
  line-height: 1.5;
  color: var(--muted);
  font-size: 13px;
}
.update-panel {
  margin-top: 20px;
  padding: 22px 24px;
  border: 1px solid var(--line);
  border-radius: 16px;
  background: var(--surface);
  display: flex;
  flex-direction: column;
  align-items: flex-start;
  gap: 10px;
}
.update-panel h3 {
  margin: 0;
}
.update-panel p {
  margin: 0;
  line-height: 1.7;
}
.update-panel pre {
  white-space: pre-wrap;
  overflow-wrap: anywhere;
  max-height: 280px;
  overflow: auto;
  font: inherit;
  color: var(--text-2);
}
.update-panel :deep(.arco-progress) {
  width: 100%;
}
.update-hash {
  overflow-wrap: anywhere;
  color: var(--muted);
}
.about-note {
  text-align: center;
  color: var(--muted);
  font-size: 12px;
  margin-top: 26px;
  line-height: 1.8;
}
@media (max-width: 760px) {
  .about-grid {
    grid-template-columns: 1fr;
  }
}
</style>
