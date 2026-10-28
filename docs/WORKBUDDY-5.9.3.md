# WorkBuddy 5.9.3 兼容性同步

从已完成审查、离线回归和打包的 WorkBuddy-Api 0.1.24，同步 WorkBuddy 5.9.3（`d4692421`）适配。解决升级官方应用后传输桥与当前加密账号导入被旧 5.9.2 门禁拒绝的问题。

本次继续同步 WorkBuddy-Api 0.1.25 的两项审查修复：原生设备 SDK 调用在等待超时后继续保持单次并发，运行循环跟随原生调用结束，迟到的有效结果可恢复缓存；补齐 `view_image` 辅助用途，允许时发送 `X-Agent-Type: auxiliary`，禁用时仍受策略限制。

SDK 超时、并发重试、迟到成功、迟到失败和恢复场景使用虚拟时钟与合成 SDK 验证；未访问真实设备服务。两个项目复用相同实现和回归用例，未修改其他 provider。

## 同步范围

- 传输桥校验 5.9.3 / CLI 2.165.0 / Node v22.21.1；继续拒绝不匹配的运行环境。加密会话入口同步更新到 5.9.3。
- User-Agent、IDE、OAuth 使用的桌面版本常量、运行状态摘要、安装提示和双语界面更新；IDE 请求头与快照版本复用同一常量。
- Rust 请求头与安装版桥回环对照改用 `src-tauri/crates/workbuddy/evidence/workbuddy-5.9.3-offline.json`，同步公开安装配置指纹。旧 5.9.2 证据和文档保留为历史记录。
- 同步版本接受/拒绝测试和加密导入回归：使用安装版 codec 加解密合成信封；实际调用当前会话入口，在门禁通过后、读取真实原生密钥之前由合成 provider 停止。

CLI、OpenAI SDK 6.25.0、设备 SDK 0.1.0 的现有接口保持不变，不升级依赖。设备 SDK 继续默认关闭、按需加载安装副本，不分发官方原生组件。

保留统一网关集成、64 MiB 请求上限、`AGENTDOCK_READ_SESSION` 入口、i18n 和 WorkBuddy crate 0.1.17，避免无关的 helper 签名指纹变化。其他 provider、现有账号与运行设置未修改。请求构造与 WorkBuddy-Api 一致，保留原有 `agent_type` 的 crate 可见性以供网关使用。

## 验证与审查

```sh
cargo test --locked --manifest-path src-tauri/Cargo.toml -p workbuddy-api --lib
cargo test --locked --manifest-path src-tauri/Cargo.toml -p agentdock --lib
node --test src-tauri/crates/workbuddy/resources/official-transport.test.cjs src-tauri/crates/workbuddy/resources/official-session.test.cjs
npm run build
git diff --check
```

WorkBuddy 模块 75 项、统一网关 89 项（另 1 项原有真实 DMG 安装测试忽略）、Node 25 项通过，前端类型与生产构建通过。已审查同步差异，未发现未解决的阻断问题。实际回归包括安装版 5.9.3 helper 启动、请求身份、gzip/SSE、SDK 开关、超时后的原生调用生命周期、读图用途策略和合成加密会话。

未调用生产聊天、真实 OAuth、续期、签到或设备令牌服务；Windows 未实机验收。本轮随后增加账号删除与批量选择功能，并构建 AgentDock 1.0.5 macOS Apple Silicon 安装包；未替换已安装应用。当前完整回归为工作区 Rust 250 项通过（1 项忽略）、Node 26 项通过，详见 [1.0.5 交付记录](releases/v1.0.5.md)。

WorkBuddy-Api 本轮安装包版本为 0.1.25；产物、校验和与审查记录位于该项目的 `releases/` 和 `docs/REVIEW-0.1.25.md`。首次适配的 0.1.24 历史产物保留，本轮为本地 ad-hoc 签名分发，未公证。

WorkBuddy-Api 当前为 0.1.26，双方账号管理与本次安装包范围见 [账号管理说明](ACCOUNT-MANAGEMENT.md)。

当前交付为 AgentDock 1.0.6 / WorkBuddy-Api 0.1.27，补齐账号删除并发边界和菜单完整显示，完整复核见 [1.0.6 记录](releases/v1.0.6.md)。
