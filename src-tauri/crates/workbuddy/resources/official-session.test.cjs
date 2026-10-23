'use strict';
const { test } = require('node:test');
const assert = require('node:assert/strict');
const { decodeSession, currentSession } = require('./official-session.cjs');

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
