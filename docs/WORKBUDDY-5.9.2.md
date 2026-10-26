# WorkBuddy 5.9.2 兼容性同步

本轮从本机 WorkBuddy-Api 的 0.1.22 迁移经过审查的反代更新；依据已安装 WorkBuddy 5.9.2（`0b5051be`）/ CLI 2.165.0 的静态配置与离线运行请求。

## 变更

- 请求头、User-Agent、IDE 版本和状态展示更新到 5.9.2 / 2.165.0；增加 `X-Agent-Runtime: other` 与关联 conversation/root ID 的 `baggage`。
- 官方桥兼容新版 CLI 的 `package.version`，继续严格核对 Node v22.21.1 与已验证安装版本。OpenAI SDK 仍为 6.25.0，无需新增或升级 npm SDK 依赖。
- 当前加密登录态读取器使用新版 `dist.js` / `workbuddy-paths2.js` 导出接口；保留 `AGENTDOCK_READ_SESSION` 调用约定，仅解密当前会话、不修改官方文件。
- 统一网关复用可独立测试的聊天发送函数：只在真实上游 HTTP 401 且尚未输出正文时续期并重试一次；保留原请求标识，只替换 Authorization。403、SSE 内错误、本地桥鉴权错误均不重放。
- 修复认证恢复顺带清空限流冷却的问题；清除登录失效状态时保留上游冷却。
- 同步脱敏离线请求、安装配置哈希及 Node/codec/回环传输回归。

保留统一网关、AgentDock 的 64 MiB 请求上限、现有日志集成及 WorkBuddy crate 0.1.17（传输 helper 签名指纹包含该版本，避免无关变动）。原生设备 SDK 仍默认关闭，显式开启后使用本机安装副本，不分发官方组件。已有账号与其他 provider 的配置未修改。

## 验证

```sh
cargo test --locked --manifest-path src-tauri/Cargo.toml -p workbuddy-api --lib
cargo test --locked --manifest-path src-tauri/Cargo.toml -p agentdock --lib
node --test src-tauri/crates/workbuddy/resources/official-transport.test.cjs src-tauri/crates/workbuddy/resources/official-session.test.cjs
npm run build
```

WorkBuddy 模块 74 项、统一网关 89 项（另 1 项原有真实 DMG 安装测试忽略）、Node 21 项通过；前端构建通过。测试包含真实安装包 codec 对合成密钥/信封的离线加解密，不读取用户原生密钥。

本轮未调用生产聊天、真实 OAuth、生产续期或签到；未重新打包或替换已安装 AgentDock。历史 shadow-wire 的 5.8.3 记录保留，当前请求对比使用 `official-desktop-offline.json` 的 5.9.2 夹具。
