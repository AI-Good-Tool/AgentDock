'use strict';
const { test } = require('node:test');
const assert = require('node:assert/strict');
const { decodeSession, currentSession } = require('./official-session.cjs');
const fs = require('node:fs');
const path = require('node:path');
const { execFileSync } = require('node:child_process');

test('installed 5.9.2 codec decodes a synthetic envelope without reading native keys or user login files', {
  skip: process.platform !== 'darwin' || !fs.existsSync('/Applications/WorkBuddy.app/Contents/Resources/app.asar'),
}, () => {
  const script = `
    const { installedCodec, decodeSession } = require(${JSON.stringify(path.join(__dirname, 'official-session.cjs'))});
    const main='/Applications/WorkBuddy.app/Contents/Resources/app.asar/main';
    const codec=installedCodec(main), runtime=require(main+'/dist.js');
    const bytes=Buffer.alloc(32,7), key={key:bytes,keyId:runtime.C(bytes)};
    const encoder=new codec.R(key);
    const envelope=encoder.seal(Buffer.from('synthetic-refresh'),{framing:'field'}).toString('base64');
    encoder.dispose();
    const result=decodeSession({auth:{accessToken:'plain',refreshToken:{$wbEncrypted:1,envelope}},account:{uid:'fixture'}},codec,key,undefined,'auth/fixture.info');
    bytes.fill(0);
    const native=process._linkedBinding('electron_browser_workbuddy_storage');
    if(result.auth.refreshToken!=='synthetic-refresh'||typeof native.loggerGet!=='function')process.exit(1);
    console.log('synthetic-codec-ok');
  `;
  assert.equal(execFileSync('/Applications/WorkBuddy.app/Contents/MacOS/Electron', ['-e', script], {
    env: { ...process.env, ELECTRON_RUN_AS_NODE: '1', AGENTDOCK_READ_SESSION: '' },
    timeout: 10000, encoding: 'utf8',
  }).trim(), 'synthetic-codec-ok');
});

// Test delegation/traversal without loading the installed client or its keys.
const codec = {
  R: class { open(value, context) { assert.equal(context.framing, 'field'); return value; } dispose() {} },
  K(bytes) { if (bytes.toString() === 'invalid') throw Error('invalid envelope'); return bytes; },
};
const encrypted = text => ({ $wbEncrypted: 1, envelope: Buffer.from(text).toString('base64') });

test('mixed plaintext/encrypted current fields decode without mutating the source', () => {
  const root = { auth: { accessToken: 'plain', refreshToken: encrypted('fixture-refresh') }, account: { uid: 'fixture', phoneNumber: encrypted('13800138000') } };
  const decoded = decodeSession(currentSession(root), codec, {}, undefined, 'auth/fixture.info');
  assert.equal(decoded.auth.accessToken, 'plain');
  assert.equal(decoded.auth.refreshToken, 'fixture-refresh');
  assert.equal(decoded.account.phoneNumber, '13800138000');
  assert.equal(typeof root.auth.refreshToken, 'object');
});

test('obsolete envelopes in unrelated accounts cannot block current-session import', () => {
  const root = { auth: { accessToken: encrypted('current') }, account: { uid: 'fixture' }, allAccounts: [{ phoneNumber: encrypted('invalid') }], accounts: [{ nickname: encrypted('invalid') }] };
  const selected = currentSession(root);
  assert.equal(decodeSession(selected, codec, {}, undefined, 'auth/fixture.info').auth.accessToken, 'current');
  assert.equal(selected.allAccounts, undefined);
  assert.equal(selected.accounts, undefined);
  root.auth.accessToken = encrypted('invalid');
  assert.throws(() => decodeSession(currentSession(root), codec, {}, undefined, 'auth/fixture.info'));
});
