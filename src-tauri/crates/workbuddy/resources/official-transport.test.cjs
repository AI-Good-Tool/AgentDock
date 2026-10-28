'use strict';
const { test } = require('node:test');
const assert = require('node:assert/strict');
const http = require('node:http');
const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');
const zlib = require('node:zlib');
const { once } = require('node:events');
const { DeviceTokens, createBridge, createMacRunLoop, runtimeHeaders, installedRoot, listenBridge, validateProfile } = require('./official-transport.cjs');

test('5.9.3 accepts CLI package.version without legacy publishConfig and rejects unverified profiles', () => {
  validateProfile({ genieVersion: '5.9.3' }, { version: '2.165.0' }, 'v22.21.1');
  validateProfile({ genieVersion: '5.9.3' }, { publishConfig: { customPackage: { version: '2.165.0' } } }, 'v22.21.1');
  for (const args of [
    [{ genieVersion: '5.8.3' }, { version: '2.161.0' }, 'v22.21.1'],
    [{ genieVersion: '5.9.2' }, { version: '2.165.0' }, 'v22.21.1'],
    [{ genieVersion: '5.9.4' }, { version: '2.165.0' }, 'v22.21.1'],
    [{ genieVersion: '5.9.3' }, { version: '2.161.0' }, 'v22.21.1'],
    [{ genieVersion: '5.9.3' }, { version: '2.165.0' }, 'v24.0.0'],
  ]) assert.throws(() => validateProfile(...args), /unsupported_profile/);
});

test('login device context requires the private key and localhost; SDK failure permits OAuth login', async t => {
  let calls = 0;
  const call = await fixture(t, { tokens: { get: async () => { calls++; throw Error('private SDK error'); } }, connect: () => { throw Error('must not reach upstream'); } });
  assert.equal((await call({}, { path: '/_auth/headers', headers: { host: 'localhost', 'x-wb-bridge-key': 'wrong' } })).status, 401);
  assert.equal((await call({}, { path: '/_auth/headers' })).status, 503);
  assert.equal(calls, 0);
  const response = await call({}, { path: '/_auth/headers', headers: { host: 'localhost' } });
  assert.equal(response.status, 200);
  assert.deepEqual(JSON.parse(response.body), { headers: {}, region: 'cn' });
  assert.equal(calls, 1);
  assert.doesNotMatch(response.body.toString(), /private SDK error/);
});

async function fixture(t, { tokens, connect } = {}) {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'wb-transport-test-'));
  const socket = path.join(dir, 't.sock');
  const server = createBridge({ key: 'test-key', variant: 'domestic', tokens: tokens || { get: async () => 'fixture-device' }, connect });
  await new Promise(resolve => listenBridge(server, { socket, tcp: true }, resolve));
  const port = server.address().port;
  t.after(() => { server.closeAllConnections(); server.close(); fs.rmSync(dir, { recursive: true, force: true }); });
  return async (body, options = {}) => {
    const wire = zlib.gzipSync(Buffer.from(JSON.stringify(body)));
    return new Promise((resolve, reject) => {
      const req = http.request({ hostname: '127.0.0.1', port, path: options.path || '/v2/chat/completions', method: 'POST',
        headers: { host: 'www.workbuddy.cn', 'x-wb-bridge-key': 'test-key', 'content-encoding': 'gzip',
          'authorization': 'Bearer fixture-user', 'x-user-id': 'fixture-uid', 'x-device-token': 'untrusted', ...options.headers } }, res => {
        const chunks = [];
        res.on('data', chunk => chunks.push(chunk));
        res.on('end', () => resolve({ status: res.statusCode, headers: res.headers, body: Buffer.concat(chunks) }));
      });
      req.on('error', reject); req.end(wire);
    });
  };
}

test('device cache is single-flight, expires at five minutes and retries with backoff', async () => {
  let now = 100; let count = 0; let fail = false;
  const tokens = new DeviceTokens(() => ({ fetchDeviceToken: async options => {
    count++; assert.deepEqual(options, { usingCachedMessage: true, includesOutdatedMessage: false, includesDeviceInfo: true, timeoutMs: 15000 });
    if (fail) throw new Error('secret must never be reported');
    return 'fixture-' + count;
  } }), () => now);
  assert.deepEqual(await Promise.all([tokens.get(), tokens.get(), tokens.get()]), ['fixture-1', 'fixture-1', 'fixture-1']);
  assert.equal(count, 1);
  now += 299999; assert.equal(await tokens.get(), 'fixture-1');
  now += 1; fail = true; await assert.rejects(tokens.get(), /^Error: device_token_unavailable$/);
  await assert.rejects(tokens.get(), /device_token_unavailable/); assert.equal(count, 2);
  now += 30000; fail = false; assert.equal(await tokens.get(), 'fixture-3');
});

test('SDK run-loop pump stops after success and failure, including concurrent waiters', async () => {
  let starts = 0, stops = 0, fail = false;
  const tokens = new DeviceTokens(() => ({ startRunLoop: () => { starts++; return () => { stops++; }; },
    fetchDeviceToken: async () => { if (fail) throw Error('fixture'); return 'fixture'; } }));
  await Promise.all([tokens.get(), tokens.get()]); assert.equal(starts, 1); assert.equal(stops, 1);
  tokens.expires = 0; fail = true; await assert.rejects(tokens.get()); assert.equal(starts, 2); assert.equal(stops, 2);
});

test('SDK retries after a watchdog timeout reuse the pending native call and its run loop', async t => {
  t.mock.timers.enable({ apis: ['setTimeout'] });
  let now = 100, starts = 0, stops = 0;
  const native = [];
  const tokens = new DeviceTokens(() => ({
    startRunLoop: () => { starts++; return () => { stops++; }; },
    fetchDeviceToken: () => new Promise(resolve => { native.push(resolve); }),
  }), () => now);
  const first = assert.rejects(tokens.get(), /device_token_unavailable/);
  const concurrent = assert.rejects(tokens.get(), /device_token_unavailable/);
  now += 15500; t.mock.timers.tick(15500);
  await Promise.all([first, concurrent]);
  assert.equal(native.length, 1); assert.equal(starts, 1); assert.equal(stops, 0);
  await assert.rejects(tokens.get(), /device_token_unavailable/);
  now += 30000;
  const retry = tokens.get();
  const waiter = tokens.get();
  assert.equal(native.length, 1); assert.equal(starts, 1);
  native[0]('late-valid-token');
  assert.deepEqual(await Promise.all([retry, waiter]), ['late-valid-token', 'late-valid-token']);
  assert.equal(stops, 1);
  assert.equal(await tokens.get(), 'late-valid-token');
  now += 300000;
  const expired = tokens.get();
  assert.equal(native.length, 2); assert.equal(starts, 2);
  native[1]('renewed-token');
  assert.equal(await expired, 'renewed-token'); assert.equal(stops, 2);
});

test('a valid native result arriving after timeout restores the cache during backoff', async t => {
  t.mock.timers.enable({ apis: ['setTimeout'] });
  let now = 100, calls = 0, stops = 0, resolveNative;
  const tokens = new DeviceTokens(() => ({
    startRunLoop: () => () => { stops++; },
    fetchDeviceToken: () => { calls++; return new Promise(resolve => { resolveNative = resolve; }); },
  }), () => now);
  const failed = assert.rejects(tokens.get(), /device_token_unavailable/);
  now += 15500; t.mock.timers.tick(15500);
  // Resolve at the timeout boundary, before the outer rejection is observed.
  resolveNative('late-cached-token');
  await failed;
  await new Promise(resolve => setImmediate(resolve));
  assert.equal(await tokens.get(), 'late-cached-token');
  assert.equal(calls, 1); assert.equal(stops, 1);
});

test('a late native rejection releases the run loop and permits a new call after backoff', async t => {
  t.mock.timers.enable({ apis: ['setTimeout'] });
  let now = 100, starts = 0, stops = 0;
  const native = [];
  const tokens = new DeviceTokens(() => ({
    startRunLoop: () => { starts++; return () => { stops++; }; },
    fetchDeviceToken: () => new Promise((resolve, reject) => { native.push({ resolve, reject }); }),
  }), () => now);
  const failed = assert.rejects(tokens.get(), /device_token_unavailable/);
  now += 15500; t.mock.timers.tick(15500); await failed;
  assert.equal(stops, 0);
  native[0].reject(new Error('late private native error'));
  await new Promise(resolve => setImmediate(resolve));
  assert.equal(stops, 1);
  await assert.rejects(tokens.get(), /device_token_unavailable/);
  now += 30000;
  const retry = tokens.get();
  assert.equal(native.length, 2); assert.equal(starts, 2);
  native[1].resolve('recovered-token');
  assert.equal(await retry, 'recovered-token'); assert.equal(stops, 2);
});

test('macOS main-loop timer is serviced by the pump with no native device SDK or network', { skip: process.platform !== 'darwin' || !fs.existsSync('/Applications/WorkBuddy.app/Contents/Resources/app.asar.unpacked/node_modules/koffi') }, async () => {
  const k = require('/Applications/WorkBuddy.app/Contents/Resources/app.asar.unpacked/node_modules/koffi');
  const cf = k.load('/System/Library/Frameworks/CoreFoundation.framework/CoreFoundation');
  const mode = cf.func('void *CFStringCreateWithCString(void *, const char *, uint32_t)')(null, 'kCFRunLoopDefaultMode', 0x08000100);
  const cbType = k.proto('void WbFixtureTimerCallback(void *, void *)');
  let fired = 0;
  const cb = k.register(() => { fired++; }, k.pointer(cbType));
  const timer = cf.func('void *CFRunLoopTimerCreate(void *, double, double, uint64_t, int64_t, void *, void *)')(
    null, cf.func('double CFAbsoluteTimeGetCurrent()')() + 0.005, 0, 0, 0, cb, null);
  cf.func('void CFRunLoopAddTimer(void *, void *, void *)')(cf.func('void *CFRunLoopGetMain()')(), timer, mode);
  const pump = createMacRunLoop(k);
  try {
    await new Promise(resolve => setTimeout(resolve, 30)); assert.equal(fired, 0);
    const stop = pump.start();
    await new Promise(resolve => setTimeout(resolve, 50)); stop(); assert.equal(fired, 1);
  } finally {
    cf.func('void CFRunLoopTimerInvalidate(void *)')(timer);
    cf.func('void CFRelease(void *)')(timer); cf.func('void CFRelease(void *)')(mode);
    k.unregister(cb); pump.close();
  }
});

test('real bridge preserves model, JSON, gzip bytes, SSE and numeric usage; secrets remain on official request only', async t => {
  const body = { model: 'deepseek-v4.1-flash', temperature: 1, messages: [{ role: 'user', content: '你好' }],
    stream: true, extra_vars: { growthEvent: 'caller-owned-event' }, tools: [{ type: 'function', function: { name: 'safe_tool' } }] };
  let observed;
  const upstream = http.createServer(async (req, res) => {
    const chunks = []; for await (const chunk of req) chunks.push(chunk);
    observed = { headers: req.headers, bytes: Buffer.concat(chunks) };
    res.writeHead(200, { 'content-type': 'text/event-stream', 'x-request-id': 'fixture-request' });
    res.write('data: {"model":"deepseek-v4.1-flash","choices":[{"delta":{"content":"OK"}}]}\n\n');
    setImmediate(() => res.end('data: {"usage":{"prompt_tokens":31,"completion_tokens":7,"total_tokens":38}}\n\ndata: [DONE]\n\n'));
  });
  upstream.listen(0, '127.0.0.1'); await once(upstream, 'listening');
  t.after(() => { upstream.closeAllConnections(); upstream.close(); });
  const call = await fixture(t, { connect: (options, callback) => {
    assert.equal(options.protocol, 'https:'); assert.equal(options.hostname, 'www.workbuddy.cn');
    assert.equal(options.headers['x-wb-bridge-key'], undefined);
    return http.request({ ...options, protocol: 'http:', hostname: '127.0.0.1', port: upstream.address().port }, callback);
  } });
  const result = await call(body, { headers: { "x-wb-device-sdk": "1" } });
  assert.equal(result.status, 200); assert.match(result.body.toString(), /"total_tokens":38/); assert.match(result.body.toString(), /\[DONE\]/);
  assert.deepEqual(observed.bytes, zlib.gzipSync(Buffer.from(JSON.stringify(body))));
  assert.deepEqual(JSON.parse(zlib.gunzipSync(observed.bytes)), body);
  assert.equal(observed.headers['content-length'], String(observed.bytes.length));
  assert.equal(observed.headers['x-device-token'], 'fixture-device');
  assert.equal(observed.headers.authorization, 'Bearer fixture-user');
  assert.equal(result.headers['x-wb-bridge-key'], undefined);
  assert.equal(result.headers['x-device-token'], undefined);
});

test('invalid origin or private authentication never contacts device SDK or upstream', async t => {
  let calls = 0;
  const call = await fixture(t, { tokens: { get: async () => { calls++; throw Error(); } }, connect: () => { calls++; throw Error(); } });
  assert.equal((await call({}, { headers: { host: 'example.com' } })).status, 503);
  assert.equal((await call({}, { path: '/other' })).status, 503);
  assert.equal((await call({}, { headers: { 'x-wb-bridge-key': 'wrong' } })).status, 401);
  assert.equal(calls, 0);
});

test('device failure is a local 503, never a silent token omission or a fallback model call', async t => {
  let calls = 0;
  const call = await fixture(t, { tokens: { get: async () => { throw Error('device_token_unavailable'); } }, connect: () => { calls++; throw Error(); } });
  const response = await call({ model: 'deepseek-v4.1-flash' }, { headers: { 'x-wb-device-sdk': '1' } });
  assert.equal(response.status, 503); assert.equal(response.headers['x-wb-local-error'], 'device_token_unavailable'); assert.equal(calls, 0);
  assert.doesNotMatch(response.body.toString(), /fixture-user|fixture-device/);
});

test('compressed payload exceeding the 64 MiB decoded cap is a local 413 before SDK or network', async t => {
  let calls = 0;
  const call = await fixture(t, { tokens: { get: async () => { calls++; return 'fixture-device'; } }, connect: () => { calls++; throw Error(); } });
  const result = await call({ messages: [{ role: 'user', content: 'x'.repeat(64 * 1024 * 1024) }] });
  assert.equal(result.status, 413);
  assert.equal(result.headers['x-wb-local-error'], 'request_too_large');
  assert.equal(JSON.parse(result.body).error.type, 'request_too_large');
  assert.match(JSON.parse(result.body).error.message, /64 MiB/);
  assert.equal(calls, 0);
});

test('five MiB payload survives bridge recompression without truncating history', async t => {
  const body = { model:'size-fixture', messages:[{role:'user',content:'x'.repeat(5*1024*1024)}] };
  let observed;
  const upstream = http.createServer(async (req,res) => {
    const chunks=[]; for await (const chunk of req) chunks.push(chunk);
    observed=JSON.parse(zlib.gunzipSync(Buffer.concat(chunks)));
    res.writeHead(200,{'content-type':'text/event-stream'});res.end('data: [DONE]\n\n');
  });
  upstream.listen(0,'127.0.0.1');await once(upstream,'listening');
  t.after(()=>{upstream.closeAllConnections();upstream.close();});
  const call=await fixture(t,{connect:(options,callback)=>http.request({...options,protocol:'http:',hostname:'127.0.0.1',port:upstream.address().port},callback)});
  assert.equal((await call(body)).status,200);
  assert.deepEqual(observed,body);
});

test('redirect response is returned intact and never followed with credentials', async t => {
  let calls = 0;
  const upstream = http.createServer((req, res) => { calls++; req.resume(); res.writeHead(307, { location: 'https://example.com/steal' }); res.end(); });
  upstream.listen(0, '127.0.0.1'); await once(upstream, 'listening');
  t.after(() => { upstream.closeAllConnections(); upstream.close(); });
  const call = await fixture(t, { connect: (options, callback) => http.request({ ...options, protocol: 'http:', hostname: '127.0.0.1', port: upstream.address().port }, callback) });
  assert.equal((await call({ model: 'deepseek-v4.1-flash' })).status, 307); assert.equal(calls, 1);
});

test('SDK defaults off, toggles per request, omits cached/injected tokens and preserves upstream rejection', async t => {
  let sdkCalls = 0, calls = 0;
  const upstream = http.createServer((req, res) => {
    calls++; req.resume();
    assert.equal(req.headers['x-wb-device-sdk'], undefined);
    assert.equal(req.headers['x-wb-bridge-key'], undefined);
    if (calls === 2) assert.equal(req.headers['x-device-token'], 'cached-fixture');
    else assert.equal(req.headers['x-device-token'], undefined);
    res.writeHead(400, { 'content-type': 'application/json' });
    res.end('{"error":"fixture-upstream-rejection"}');
  });
  upstream.listen(0, '127.0.0.1'); await once(upstream, 'listening');
  t.after(() => { upstream.closeAllConnections(); upstream.close(); });
  const call = await fixture(t, {
    tokens: { get: async () => { sdkCalls++; return 'cached-fixture'; } },
    connect: (options, cb) => http.request({ ...options, protocol: 'http:', hostname: '127.0.0.1', port: upstream.address().port }, cb),
  });
  for (const mode of [undefined, '1', '0', 'true']) {
    const response = await call({ model: 'fixture' }, { headers: mode ? { 'x-wb-device-sdk': mode } : {} });
    assert.equal(response.status, 400);
    assert.equal(response.body.toString(), '{"error":"fixture-upstream-rejection"}');
  }
  assert.equal(sdkCalls, 1); assert.equal(calls, 4);
});

test('startup preflight requires private authentication, warms shared token cache and never calls chat upstream', async t => {
  let loads = 0, fetches = 0, network = 0;
  const tokens = new DeviceTokens(() => {
    loads++;
    return { fetchDeviceToken: async () => { fetches++; return 'private-startup-token'; } };
  });
  const call = await fixture(t, { tokens, connect: () => { network++; throw Error(); } });
  const options = { path: '/_device/prepare', headers: { host: 'localhost' } };
  assert.equal((await call({}, { ...options, headers: { host: 'localhost', 'x-wb-bridge-key': 'invalid' } })).status, 401);
  assert.equal(loads, 0);
  const ready = await call({}, options);
  assert.equal(ready.status, 204); assert.equal(ready.body.length, 0);
  assert.equal((await call({}, options)).status, 204);
  assert.equal(await tokens.get(), 'private-startup-token');
  assert.equal(loads, 1); assert.equal(fetches, 1); assert.equal(network, 0);
});

test('startup authorization failure is explicit, never reports ready or sends model traffic', async t => {
  let network = 0;
  const call = await fixture(t, {
    tokens: { get: async () => { throw Error('private-sdk-error'); } },
    connect: () => { network++; throw Error(); },
  });
  const result = await call({}, { path: '/_device/prepare', headers: { host: 'localhost' } });
  assert.equal(result.status, 503);
  assert.equal(result.headers['x-wb-local-error'], 'device_token_unavailable');
  assert.doesNotMatch(result.body.toString(), /private-sdk-error/);
  assert.equal(network, 0);
});

test('SDK off routes both official regions, SDK on never sends a mismatched regional token', async t => {
  let sdk = 0, network = 0;
  const upstream = http.createServer((req, res) => {
    req.resume(); assert.equal(req.headers['x-device-token'], undefined); res.writeHead(200); res.end('fixture');
  });
  upstream.listen(0, '127.0.0.1'); await once(upstream, 'listening');
  t.after(() => { upstream.closeAllConnections(); upstream.close(); });
  const call = await fixture(t, {
    tokens: { get: async () => { sdk++; return 'fixture'; } },
    connect: (options, cb) => {
      network++; assert.equal(options.hostname, 'www.workbuddy.ai');
      return http.request({ ...options, protocol: 'http:', hostname: '127.0.0.1', port: upstream.address().port }, cb);
    },
  });
  assert.equal((await call({}, { headers: { host: 'www.workbuddy.ai' } })).status, 200);
  const mismatch = await call({}, { headers: { host: 'www.workbuddy.ai', 'x-wb-device-sdk': '1' } });
  assert.equal(mismatch.status, 503); assert.equal(mismatch.headers['x-wb-local-error'], 'sdk_region_unavailable');
  assert.equal(sdk, 0); assert.equal(network, 1);
});


test('installed root follows Windows executable location, including spaces and custom drives', () => {
  assert.equal(installedRoot('win32', 'D:\\Custom Apps\\WorkBuddy\\WorkBuddy.exe'),
    'D:\\Custom Apps\\WorkBuddy\\resources\\app.asar.unpacked');
  assert.equal(installedRoot('darwin', '/unused/helper'),
    '/Applications/WorkBuddy.app/Contents/Resources/app.asar.unpacked');
  assert.throws(() => installedRoot('linux', '/tmp/node'), /unsupported_runtime/);
});

test('portable bridge binds loopback only and publishes an ephemeral valid port', async t => {
  const server = createBridge({key:'fixture', variant:'domestic', tokens:{get:async()=>{throw Error('unexpected SDK')}}});
  const ready = await new Promise(resolve => listenBridge(server, {tcp:true}, resolve));
  t.after(()=>{server.closeAllConnections();server.close()});
  assert.equal(server.address().address, '127.0.0.1');
  assert.ok(Number.isInteger(ready.port) && ready.port > 0 && ready.port <= 65535);
  const response = await new Promise((resolve,reject)=>{
    http.get({hostname:'127.0.0.1',port:ready.port,path:'/_device/prepare'}, res=>{
      res.resume();res.on('end',()=>resolve(res.statusCode));
    }).on('error',reject);
  });
  assert.equal(response,401);
});


test('runtime headers use real OS/architecture rather than inheriting a macOS-only profile', () => {
  assert.deepEqual(runtimeHeaders('win32', 'x64'), {'x-stainless-os':'Windows','x-stainless-arch':'x64'});
  assert.deepEqual(runtimeHeaders('darwin', 'arm64'), {'x-stainless-os':'MacOS','x-stainless-arch':'arm64'});
});
