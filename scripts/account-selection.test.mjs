import test from 'node:test'
import assert from 'node:assert/strict'
import { visibleSelection, toggleSelection, accountKey } from '../src/lib/account-selection.ts'
test('batches exclude hidden/deleted accounts and duplicate ids', () => {
  assert.deepEqual(visibleSelection(['cn:a', 'ai:b', 'cn:a', 'gone'], ['cn:a']), ['cn:a'])
  assert.deepEqual(visibleSelection(['cn:a'], []), [])
  assert.deepEqual(toggleSelection(['a'], 'a', true), ['a'])
  assert.deepEqual(toggleSelection(['a', 'b'], 'a', false), ['b'])
  assert.notEqual(accountKey('workbuddy', 'same'), accountKey('trae', 'same'))
  assert.notEqual(accountKey('a/b', 'c'), accountKey('a', 'b/c'))
})
