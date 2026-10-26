'use strict';
// Read only. Use the installed client's codec and native key provider, never copy keys,
// create keyblobs, change its login file, or fall back to a different saved session.
const fs = require('node:fs');
const path = require('node:path');

function decodeSession(root, crypto, key, userKey, resourceId) {
  const codec = new crypto.R(key);
  function visit(value, parts = []) {
    if (!value || typeof value !== 'object') return value;
    if (value.$wbEncrypted === 1) {
      const bytes = Buffer.from(value.envelope, 'base64');
      if (value.scheme === 'asym-v1') {
        if (!userKey) throw new Error('missing_local_key');
        const fieldPath = '/' + parts.map(p => /^\d+$/.test(p) ? '*' : p).join('/');
        return crypto.I(bytes, { framing: 'field', purpose: 'workbuddy-auth-session', resourceId, fieldPath }, { userKey, developerPublicKey: crypto.developerPublicKey }).toString('utf8');
      }
      return codec.open(crypto.K(bytes), { framing: 'field' }).toString('utf8');
    }
    return Array.isArray(value) ? value.map((v, i) => visit(v, [...parts, String(i)]))
      : Object.fromEntries(Object.entries(value).map(([k, v]) => [k, visit(v, [...parts, k])]));
  }
  try { return visit(root); } finally { codec.dispose(); }
}

function currentSession(root) { return { auth: root.auth, account: root.account }; }

// 5.9.2 split the old app-lifetime-job crypto chunk into dist.js. Keep
// minified exports confined to this adapter; no application bootstrap is run.
function installedCodec(main) {
  const runtime = require(path.join(main, 'dist.js'));
  runtime.s(); runtime.h();
  return {
    R: runtime.v, K: runtime.E, I: runtime.g, G: runtime.T,
    A: runtime.u, k: runtime.l,
  };
}

function readInstalledSession(executable, authPath) {
  const resources = process.platform === 'darwin'
    ? path.resolve(executable, '../../Resources') : path.join(path.dirname(executable), 'resources');
  const main = path.join(resources, 'app.asar/main');
  const product = JSON.parse(fs.readFileSync(path.join(resources, 'app.asar.unpacked/cli/product.json'), 'utf8'));
  if (product.genieVersion !== '5.9.2') throw new Error('unsupported_profile');
  const crypto = installedCodec(main);
  const native = process._linkedBinding('electron_browser_workbuddy_storage');
  const capabilities = crypto.G(native.loggerGet());
  if (capabilities.symmetric.status !== 'available') throw new Error('local_key_unavailable');
  const key = capabilities.symmetric.value;
  crypto.developerPublicKey = capabilities.developerPublicKey.value;
  let userKey;
  try {
    const paths = require(path.join(main, 'workbuddy-paths2.js'));
    paths.h();
    const master = crypto.A({ keyblobPath: crypto.k(paths.c()), protectorKey: key, createIfMissing: false });
    if (master.status === 'available') userKey = master.value;
    if (fs.existsSync(authPath + '.logged-out')) throw new Error('logged_out');
    const root = JSON.parse(fs.readFileSync(authPath, 'utf8'));
    // Unrelated old accounts may have obsolete envelopes; they are not part of
    // importing the current login and must neither block it nor be decrypted.
    return decodeSession(currentSession(root), crypto, key, userKey, 'auth/' + path.basename(authPath));
  } finally { key.key.fill(0); userKey?.key.fill(0); }
}

if (require.main === module || process.env.AGENTDOCK_READ_SESSION === '1') {
  try { process.stdout.write(JSON.stringify(readInstalledSession(process.execPath, process.argv[1]))); }
  catch { process.stdout.write('{"error":"无法读取官方加密登录态，请确认本机 WorkBuddy 已登录且版本受支持"}'); process.exitCode = 1; }
}
module.exports = { decodeSession, readInstalledSession, currentSession, installedCodec };
