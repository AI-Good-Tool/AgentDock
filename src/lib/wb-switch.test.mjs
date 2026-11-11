import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import test from 'node:test'
import { runInNewContext } from 'node:vm'
import ts from 'typescript'

const source = readFileSync(new URL('./wb-switch.ts', import.meta.url), 'utf8')
const { outputText } = ts.transpileModule(source, {
  compilerOptions: { module: ts.ModuleKind.CommonJS },
})
const exports = {}
runInNewContext(outputText, { exports })
const { canCopySession, syncMode, directSync, syncSelections, sessionKey, migrationIssues } =
  exports
const group = (overrides) => ({
  groupId: 'g',
  previewToken: 'live-token',
  availableModes: ['fastForward'],
  ...overrides,
})

test('safe bulk sync excludes conflicts, archive-only and unverifiable previews', () => {
  assert.equal(directSync(group({})), true)
  assert.equal(directSync(group({ availableModes: ['overwrite'] })), false)
  assert.equal(directSync(group({ availableModes: [], archiveAction: 'statusOnly' })), false)
  assert.equal(directSync(group({ previewToken: undefined })), false)
})
test('selection carries the exact preview token and backend-authorized mode', () => {
  const groups = [
    group({}),
    group({ groupId: 'conflict', availableModes: ['overwrite'] }),
    group({ groupId: 'invalid', previewToken: undefined }),
  ]
  const result = JSON.parse(
    JSON.stringify(syncSelections(groups, new Set(['g', 'conflict', 'invalid'])))
  )
  assert.deepEqual(result, [
    { groupId: 'g', previewToken: 'live-token', mode: 'fastForward' },
    { groupId: 'conflict', previewToken: 'live-token', mode: 'overwrite' },
  ])
  assert.equal(syncSelections(groups, new Set()).length, 0)
})
test('archive-only sync requires a preview token and preserves body mode priority', () => {
  assert.equal(syncMode(group({ availableModes: [], archiveAction: 'statusOnly' })), 'statusOnly')
  assert.equal(syncMode(group({ archiveAction: 'statusOnly' })), 'fastForward')
  assert.equal(
    syncMode(group({ availableModes: [], archiveAction: 'statusOnly', previewToken: '' })),
    null
  )
  assert.equal(syncMode(group({ availableModes: [] })), null)
})
test('IDE and VS Code selections distinguish identical IDs in different workspaces', () => {
  assert.notEqual(
    sessionKey({ id: 'same', workspaceHash: 'one' }),
    sessionKey({ id: 'same', workspaceHash: 'two' })
  )
})
test('partial migration errors, skipped writes and recovery leftovers remain visible', () => {
  const issues = migrationIssues({
    ok: true,
    sessionCopy: { error: 'copy failed', linkErrors: ['link failed'] },
    sessionSync: {
      errors: [{ groupId: 'g', error: 'changed' }],
      skipped: [{ id: 's', reason: 'stale preview' }],
    },
    sessionRecovery: {
      needsRecovery: [{ operationId: 'op', reason: 'recover' }],
      temporaryFiles: [{ title: 'session', reason: 'cleanup pending' }],
    },
  })
  assert.equal(issues.length, 6)
  assert.ok(issues.includes('g：changed'))
  assert.ok(issues.includes('op：recover'))
})

test('boolean recovery flags cannot break result rendering', () => {
  const issues = migrationIssues(
    {
      sessionCopy: { needsRecovery: true },
      sessionSync: { needsRecovery: false, errors: null, skipped: 0 },
      sessionRecovery: {
        needsRecovery: [{ operationId: 'op', reason: 'retry' }],
        temporaryFiles: [null],
      },
    },
    'Recovery required'
  )
  assert.equal(issues.length, 2)
  assert.ok(issues.includes('Recovery required'))
  assert.ok(issues.includes('op：retry'))
})

test('IDE copy failures retain their conversation identifier', () => {
  const issues = migrationIssues({
    sessionCopy: { errors: [{ conversationId: 'conversation-1', error: 'Missing history' }] },
  })
  assert.equal(issues[0], 'conversation-1：Missing history')
})

test('copy selection requires supported storage, a different source account and local history', () => {
  const preview = { sourceUid: 'source' }
  assert.equal(canCopySession(preview, { hasHistory: true }), true)
  assert.equal(canCopySession({ ...preview, supported: false }, { hasHistory: true }), false)
  assert.equal(canCopySession({ ...preview, sameAccount: true }, { hasHistory: true }), false)
  assert.equal(canCopySession({}, { hasHistory: true }), false)
  assert.equal(canCopySession(preview, {}), false)
  assert.equal(canCopySession(preview, { hasHistory: false }), false)
})
