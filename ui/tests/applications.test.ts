import assert from 'node:assert/strict'
import test from 'node:test'

import {
  filterApplications,
  getApplicationCardStates,
  getApplicationAccent,
  getApplicationInitial,
  hasApplicationIcon,
  type Application,
  validateApplications,
} from '../src/applications.js'

const applications: Application[] = [
  { id: 'firefox', name: 'Firefox', iconUri: 'file:///icons/firefox.png' },
  { id: 'terminal', name: 'System Terminal', iconUri: null },
  { id: 'files', name: 'Files' },
]

test('validateApplications requires the native result to be an array', () => {
  assert.throws(
    () => validateApplications({ applications: [] }),
    /Launcher\.getApplications\(\) must resolve to an array/,
  )
})

test('validateApplications requires every array item to be an object', () => {
  assert.throws(
    () => validateApplications([null]),
    /item at index 0 must be an object/,
  )
  assert.throws(
    () => validateApplications([[]]),
    /item at index 0 must be an object/,
  )
})

test('validateApplications requires a non-empty string id', () => {
  for (const id of [undefined, 42, '   ']) {
    assert.throws(
      () => validateApplications([{ id, name: 'Files' }]),
      /item at index 0 has an invalid id; expected a non-empty string/,
    )
  }
})

test('validateApplications requires a non-empty string name', () => {
  for (const name of [undefined, false, '   ']) {
    assert.throws(
      () => validateApplications([{ id: 'files', name }]),
      /item at index 0 has an invalid name; expected a non-empty string/,
    )
  }
})

test('validateApplications rejects unsupported icon URI values', () => {
  assert.throws(
    () => validateApplications([{ id: 'files', name: 'Files', iconUri: 42 }]),
    /item at index 0 has an invalid iconUri; expected a string, null, or undefined/,
  )
})

test('validateApplications rejects duplicate application IDs', () => {
  assert.throws(
    () => validateApplications([
      { id: 'files', name: 'Files' },
      { id: 'terminal', name: 'Terminal' },
      { id: 'files', name: 'Another Files' },
    ]),
    /duplicate id "files" at index 2; first seen at index 0/,
  )
})

test('validateApplications accepts every supported icon URI shape', () => {
  const nativeResult = [
    { id: 'files', name: 'Files', iconUri: 'file:///icons/files.png' },
    { id: 'terminal', name: 'Terminal', iconUri: null },
    { id: 'browser', name: 'Browser', iconUri: undefined },
    { id: 'settings', name: 'Settings' },
  ]

  assert.deepEqual(validateApplications(nativeResult), nativeResult)
})

test('filterApplications matches case-insensitively and trims the query', () => {
  assert.deepEqual(filterApplications(applications, '  TERMINAL '), [
    applications[1],
  ])
})

test('filterApplications preserves the original list for an empty query', () => {
  assert.equal(filterApplications(applications, '   '), applications)
})

test('filterApplications returns an empty list when nothing matches', () => {
  assert.deepEqual(filterApplications(applications, 'calendar'), [])
})

test('getApplicationCardStates keeps every card while assigning visible positions', () => {
  const states = getApplicationCardStates(applications, 'terminal')

  assert.deepEqual(
    states.map(({ application, visible, visiblePosition }) => ({
      id: application.id,
      visible,
      visiblePosition,
    })),
    [
      { id: 'firefox', visible: false, visiblePosition: null },
      { id: 'terminal', visible: true, visiblePosition: 1 },
      { id: 'files', visible: false, visiblePosition: null },
    ],
  )
})

test('getApplicationCardStates reports no visible cards without removing them', () => {
  const states = getApplicationCardStates(applications, 'calendar')

  assert.equal(states.length, applications.length)
  assert.equal(states.filter(state => state.visible).length, 0)
  assert.deepEqual(states.map(state => state.application.id), [
    'firefox',
    'terminal',
    'files',
  ])
})

test('getApplicationInitial uses the first visible character', () => {
  assert.equal(getApplicationInitial('  firefox'), 'F')
  assert.equal(getApplicationInitial('Lynx'), 'L')
  assert.equal(getApplicationInitial(''), '?')
  assert.equal(getApplicationInitial('   '), '?')
})

test('hasApplicationIcon rejects missing and blank icon URIs', () => {
  assert.equal(hasApplicationIcon(applications[0]), true)
  assert.equal(hasApplicationIcon(applications[1]), false)
  assert.equal(hasApplicationIcon(applications[2]), false)
  assert.equal(
    hasApplicationIcon({ id: 'blank', name: 'Blank', iconUri: '   ' }),
    false,
  )
})

test('getApplicationAccent is stable and returns a palette index', () => {
  const accent = getApplicationAccent(applications[0])

  assert.equal(getApplicationAccent(applications[0]), accent)
  assert.ok(accent >= 0 && accent <= 3)
})
