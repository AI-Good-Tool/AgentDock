'use strict';
// Own transport only. The installed native SDK is loaded lazily and never copied.
const http = require('node:http');
const https = require('node:https');
const fs = require('node:fs');
const path = require('node:path');
const zlib = require('node:zlib');
const { pipeline } = require('node:stream');

function runtimeHeaders(platform, arch) {
  return { 'x-stainless-os': ({ darwin: 'MacOS', win32: 'Windows', linux: 'Linux' })[platform] || 'Unknown',
    'x-stainless-arch': arch };
}
function encode(body) {
  return zlib.gzipSync(Buffer.from(JSON.stringify(body)));
}

class DeviceTokens {
  constructor(load, now = Date.now) {
    this.load = load;
    this.now = now;
    this.value = undefined;
    this.expires = 0;
    this.retryAt = 0;
    this.failures = 0;
  }
  async get() {
    if (this.value && this.now() < this.expires) return this.value;
    if (this.inFlight) return this.inFlight;
    if (this.now() < this.retryAt) throw new Error('device_token_unavailable');
    this.inFlight = this.refresh().finally(() => { this.inFlight = undefined; });
    return this.inFlight;
  }
  async refresh() {
    let timeout;
    let stopRunLoop;
    try {
      this.sdk ??= this.load();
      stopRunLoop = this.sdk.startRunLoop?.();
      const token = await Promise.race([
        this.sdk.fetchDeviceToken({ usingCachedMessage: true, includesOutdatedMessage: false,
          includesDeviceInfo: true, timeoutMs: 15000 }),
        new Promise((_, reject) => { timeout = setTimeout(() => reject(new Error('timeout')), 15500); }),
      ]);
      if (typeof token !== 'string' || !token.trim() || /[\r\n]/.test(token)) throw new Error('invalid token');
      this.value = token;
      this.expires = this.now() + 300000;
      this.failures = 0;
      this.retryAt = 0;
      return token;
    } catch {
      this.value = undefined;
      this.retryAt = this.now() + Math.min(300000, 30000 * 2 ** Math.min(this.failures++, 4));
      throw new Error('device_token_unavailable');
    } finally { clearTimeout(timeout); stopRunLoop?.(); }
  }
}

// ELECTRON_RUN_AS_NODE runs libuv but does not service the macOS main CFRunLoop.
// The installed native SDK links AppKit/WebKit and schedules main-loop work.
// Pump only while a device request is outstanding; no background token refresh.
function createMacRunLoop(koffi) {
  const cf = koffi.load('/System/Library/Frameworks/CoreFoundation.framework/CoreFoundation');
  const mode = cf.func('void *CFStringCreateWithCString(void *, const char *, uint32_t)')(null, 'kCFRunLoopDefaultMode', 0x08000100);
  const run = cf.func('int32_t CFRunLoopRunInMode(void *, double, bool)');
  const release = cf.func('void CFRelease(void *)');
  let timer;
  return {
    start() {
      timer ??= setInterval(() => run(mode, 0.001, true), 10);
      timer.unref();
      return () => { clearInterval(timer); timer = undefined; };
    },
    close() { clearInterval(timer); timer = undefined; release(mode); },
  };
}

// A Cocoa-capable Node helper must remain a background process. Loading AppKit
// with the official executable's bundle identity otherwise registers a second
// WorkBuddy Dock application which never completes its normal app startup.
let cocoaPolicy;
function initializeBackgroundCocoa(koffi) {
  if (cocoaPolicy) return cocoaPolicy();
  const services = koffi.load('/System/Library/Frameworks/ApplicationServices.framework/ApplicationServices');
  const psn = Buffer.alloc(8);
  psn.writeUInt32LE(2, 4); // { highLongOfPSN: 0, lowLongOfPSN: kCurrentProcess }
  const transform = services.func('int32_t TransformProcessType(const void *, uint32_t)');
  transform(psn, 2); // LSBackgroundOnly launchers are already background-only.
  const appkit = koffi.load('/System/Library/Frameworks/AppKit.framework/AppKit');
  if (!appkit.func('bool NSApplicationLoad()')()) throw new Error('cocoa_unavailable');
  const objc = koffi.load('/usr/lib/libobjc.A.dylib');
  const klass = objc.func('void *objc_getClass(const char *)');
  const selector = objc.func('void *sel_registerName(const char *)');
  const object = objc.func('objc_msgSend', 'void *', ['void *', 'void *']);
  const setPolicy = objc.func('objc_msgSend', 'bool', ['void *', 'void *', 'int64_t']);
  const app = object(klass('NSApplication'), selector('sharedApplication'));
  setPolicy(app, selector('setActivationPolicy:'), 2); // Already-background applications may return false.
  objc.func('objc_msgSend', 'void', ['void *', 'void *'])(app, selector('finishLaunching'));
  const getPolicy = objc.func('objc_msgSend', 'int64_t', ['void *', 'void *']);
  cocoaPolicy = () => Number(getPolicy(app, selector('activationPolicy')));
  return cocoaPolicy();
}

function installedRoot(platform, executable) {
  if (platform === 'darwin') return '/Applications/WorkBuddy.app/Contents/Resources/app.asar.unpacked';
  if (platform === 'win32') return path.win32.join(path.win32.dirname(executable), 'resources', 'app.asar.unpacked');
  throw new Error('unsupported_runtime');
}

// Windows cannot use Unix sockets; the port is assigned by the OS, never predicted.
function listenBridge(server, { socket, tcp }, ready) {
  if (tcp) return server.listen(0, '127.0.0.1', () => ready({ port: server.address().port }));
  if (!socket) throw new Error('missing_configuration');
  return server.listen(socket, () => { fs.chmodSync(socket, 0o600); ready({}); });
}
function installedProfile() {
  if (!['darwin', 'win32'].includes(process.platform) || process.version !== 'v22.21.1') throw new Error('unsupported_runtime');
  const root = installedRoot(process.platform, process.execPath);
  const product = JSON.parse(fs.readFileSync(path.join(root, 'cli/product.json'), 'utf8'));
  const cli = JSON.parse(fs.readFileSync(path.join(root, 'cli/package.json'), 'utf8'));
  const variant = product.isOversea ? 'overseas' : 'domestic';
  if (product.genieVersion !== '5.8.3' || cli.publishConfig?.customPackage?.version !== '2.161.0') throw new Error('unsupported_profile');
  const channel = product.config?.turingSdk?.channelId;
  return { variant, load() {
    const sdkVariant = JSON.parse(fs.readFileSync(path.join(root, 'native/turing-sdk/build/Release/turing-sdk-variant.json'), 'utf8')).variant;
    if (!Number.isInteger(channel) || channel <= 0 || channel > 2147483647 || sdkVariant !== variant) throw new Error('unsupported_sdk');
    let loop;
    if (process.platform === 'darwin') {
      const koffi = require(path.join(root, 'node_modules/koffi'));
      if (initializeBackgroundCocoa(koffi) !== 2) throw new Error('background_policy_unavailable');
      loop = createMacRunLoop(koffi);
    }
    try {
      const sdk = require(path.join(root, 'native/turing-sdk/index.cjs'));
      if (!sdk.isSupported()) throw new Error('sdk_unavailable');
      sdk.configure(channel, product.applicationName || product.productName, product.productVersion || product.genieVersion, process.platform === 'darwin');
      return { fetchDeviceToken: options => sdk.fetchDeviceToken(options), startRunLoop: () => loop?.start() };
    } catch (error) { loop?.close(); throw error; }
  }};
}

const HOP_HEADERS = new Set(['host', 'connection', 'content-length', 'transfer-encoding', 'keep-alive',
  'proxy-authenticate', 'proxy-authorization', 'te', 'trailer', 'upgrade', 'x-wb-bridge-key', 'x-wb-device-sdk', 'x-device-token']);
function createBridge({ key, tokens, variant, connect = https.request }) {
  const sdkHost = variant === 'overseas' ? 'www.workbuddy.ai' : 'www.workbuddy.cn';
  return http.createServer(async (req, res) => {
    const fail = (status, code) => {
      if (!res.headersSent && !res.destroyed) {
        res.writeHead(status, { 'content-type': 'application/json', 'x-wb-local-error': code });
        res.end(JSON.stringify({ error: { type: code, message: code === 'device_token_unavailable'
          ? '官方设备 SDK 暂未取得有效令牌，请检查 WorkBuddy 安装与设备服务状态后重试'
          : code === 'sdk_region_unavailable' ? '设备 SDK 与目标账号区域不匹配，请关闭设备 SDK 后使用跨地区反代'
          : code === 'request_too_large' ? '请求体超过本地 64 MiB 上限，请缩短会话历史或减少附件后重试'
          : '本机官方请求传输不可用' } }));
      } else res.destroy();
    };
    if (req.headers['x-wb-bridge-key'] !== key) { req.resume(); return fail(401, 'bridge_auth_failed'); }
    // Private OAuth context; unavailable SDK data is omitted just as in the desktop.
    if (req.method === 'POST' && req.url === '/_auth/headers' && req.headers.host === 'localhost') {
      req.resume();
      const headers = {};
      try { headers['X-Device-Token'] = await tokens.get(); } catch { /* best effort */ }
      res.writeHead(200, { 'content-type': 'application/json' });
      res.end(JSON.stringify({ headers, region: variant === 'overseas' ? 'ai' : 'cn' }));
      return;
    }
    // Private startup preflight: no chat request or credential is sent upstream.
    if (req.method === 'POST' && req.url === '/_device/prepare' && req.headers.host === 'localhost') {
      req.resume();
      try {
        await tokens.get();
        if (!res.destroyed) { res.writeHead(204); res.end(); }
      } catch { fail(503, 'device_token_unavailable'); }
      return;
    }
    const allowed = req.headers.host;
    if (req.method !== 'POST' || req.url !== '/v2/chat/completions' ||
        !['www.workbuddy.cn', 'www.workbuddy.ai'].includes(allowed)) {
      req.resume(); return fail(503, 'official_profile_unavailable');
    }
    // Without SDK, both official regions are supported. Never reuse one region's SDK token in another.
    if (req.headers['x-wb-device-sdk'] === '1' && allowed !== sdkHost) {
      req.resume(); return fail(503, 'sdk_region_unavailable');
    }
    let outbound;
    res.on('close', () => { outbound?.destroy(); });
    req.on('aborted', () => { outbound?.destroy(); });
    try {
      const chunks = []; let size = 0;
      for await (const chunk of req) {
        size += chunk.length;
        if (size > 64 * 1024 * 1024) return fail(413, 'request_too_large');
        chunks.push(chunk);
      }
      const bytes = Buffer.concat(chunks);
      const plain = req.headers['content-encoding'] === 'gzip'
        ? zlib.gunzipSync(bytes, { maxOutputLength: 64 * 1024 * 1024 }) : bytes;
      const body = JSON.parse(plain);
      const wire = encode(body);
      // Opt-in only. Never load native SDK, use a cached token or auto-fallback when off.
      const device = req.headers['x-wb-device-sdk'] === '1' ? await tokens.get() : undefined;
      if (res.destroyed) return;
      // Header order follows the observed desktop SDK profile, not Rust's map ordering.
      const order = ['accept', 'content-type', 'x-requested-with', 'x-stainless-arch', 'x-stainless-lang',
        'x-stainless-os', 'x-stainless-package-version', 'x-stainless-retry-count', 'x-stainless-runtime',
        'x-stainless-runtime-version', 'x-conversation-id', 'x-agent-intent', 'x-agent-purpose', 'x-ide-type',
        'x-ide-name', 'x-ide-version', 'x-private-data', 'acp-connection-id', 'x-codebuddy-request',
        'x-device-token', 'x-request-id', 'x-conversation-message-id', 'x-conversation-request-id',
        'x-root-request-id', 'x-agent-type', 'content-encoding', 'traceparent', 'b3', 'x-b3-traceid',
        'x-b3-parentspanid', 'x-b3-spanid', 'x-b3-sampled', 'x-trace-id', 'x-product', 'user-agent',
        'authorization', 'x-user-id', 'x-domain', 'x-enterprise-id', 'x-tenant-id', 'content-length', 'accept-encoding'];
      const source = { ...req.headers, ...runtimeHeaders(process.platform, process.arch), 'x-device-token': device, 'content-encoding': 'gzip', 'content-length': wire.length };
      const headers = {};
      for (const name of order) if (source[name] !== undefined) headers[name] = source[name];
      outbound = connect({ protocol: 'https:', hostname: allowed, path: req.url, method: 'POST', headers }, upstream => {
        const responseHeaders = {};
        for (const [name, value] of Object.entries(upstream.headers)) {
          if (!HOP_HEADERS.has(name) && name !== 'x-wb-local-error') responseHeaders[name] = value;
        }
        res.writeHead(upstream.statusCode || 502, responseHeaders);
        pipeline(upstream, res, () => {});
      });
      outbound.on('error', () => fail(502, 'official_transport_error'));
      outbound.setTimeout(180000, () => outbound.destroy());
      outbound.end(wire);
    } catch (e) {
      if (e?.code === 'ERR_BUFFER_TOO_LARGE') return fail(413, 'request_too_large');
      fail(503, e?.message === 'device_token_unavailable' ? e.message : 'official_transport_error');
    }
  });
}

if (require.main === module || process.env.WB_OFFICIAL_BRIDGE === '1') {
  try {
    const profile = installedProfile();
    const socket = process.env.WB_OFFICIAL_SOCKET;
    const key = process.env.WB_OFFICIAL_KEY;
    const tcp = process.platform === 'win32' && process.env.WB_OFFICIAL_TCP === '1';
    if ((!socket && !tcp) || !key) throw new Error('missing_configuration');
    const server = createBridge({ key, tokens: new DeviceTokens(profile.load), variant: profile.variant });
    server.on('error', () => process.exit(1));
    listenBridge(server, { socket, tcp }, endpoint => {
      process.stdout.write(JSON.stringify({ ready: true, node: process.version, variant: profile.variant, ...endpoint }) + '\n');
    });
    process.stdin.resume();
    process.stdin.on('end', () => { server.closeAllConnections(); server.close(() => process.exit(0)); setTimeout(() => process.exit(0), 1000).unref(); });
  } catch { process.stdout.write('{"ready":false}\n'); process.exitCode = 1; }
}
module.exports = { runtimeHeaders, installedRoot, listenBridge, encode, DeviceTokens, createBridge, installedProfile, createMacRunLoop, initializeBackgroundCocoa };
