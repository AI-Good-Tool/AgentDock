import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import test from 'node:test'
import { runInNewContext } from 'node:vm'
import ts from 'typescript'

// Exercise the production formatters without mounting the desktop/i18n runtime.
const source = readFileSync(new URL('./account-display.ts', import.meta.url), 'utf8')
const { outputText } = ts.transpileModule(source, {
  compilerOptions: { module: ts.ModuleKind.CommonJS },
})
const exports = {}
runInNewContext(outputText, { exports, require: () => ({ t: zh => zh, isZh: { value: true } }) })
const { accountName, phoneText } = exports
const fixture = overrides => ({ name: '演示账号', presentation: { phoneNumber: '13800138000' }, ...overrides })

test('inherited phone display name obeys the reveal switch', () => {
  const account = fixture({ displayField: '', presentation: { displayName: '+86 13800138000', phoneNumber: '13800138000' } })
  assert.equal(accountName(account, false), '138****8000')
  assert.equal(accountName(account, true), '+86 13800138000')
  assert.equal(accountName(account), phoneText(account.presentation.phoneNumber, false))
})

test('explicit phone titles and legacy phone-only names stay private', () => {
  const account = fixture({ displayField: 'phone' })
  assert.equal(accountName(account), '138****8000')
  assert.equal(accountName(account, true), '13800138000')
  const legacy = fixture({ displayField: 'phone', name: '13800138000', presentation: {} })
  assert.equal(accountName(legacy), '138****8000')
  assert.equal(accountName(legacy, true), '13800138000')
})

test('nickname and note fallbacks cannot bypass phone hiding', () => {
  assert.equal(accountName(fixture({ displayField: 'note', note: '13800138000' })), '138****8000')
  assert.equal(accountName(fixture({ displayField: 'nickname', presentation: { nickname: '13800138000' } })), '138****8000')
  assert.equal(accountName(fixture({ displayField: 'note', note: '工作账号 123' })), '工作账号 123')
  assert.equal(accountName(fixture()), '演示账号')
})

test('source-masked numbers are not made up when reveal is enabled', () => {
  const account = fixture({ displayField: 'phone', presentation: { phoneNumber: '138****8000' } })
  assert.equal(accountName(account, true), '138****8000')
  assert.equal(accountName(account), '138****8000')
})
