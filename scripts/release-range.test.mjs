// node --test scripts/release-range.test.mjs

import { test } from 'node:test';
import assert from 'node:assert/strict';
import { compareTags, parseTag, previousTag } from './lib/release-range.mjs';

const TAGS = [
  'v0.1.60',
  'v0.1.61-alpha.1',
  'v0.1.61',
  'v0.1.62-alpha.1',
  'v0.1.62-alpha.2',
  'v0.1.62-rc.1',
  'tray-v0.1.29-alpha.1',
  'tray-v0.1.29',
  'tray-v0.1.30',
  'tray-v0.1.31-alpha.1',
  'not-a-release',
];

test('parseTag reads track, version and channel', () => {
  assert.deepEqual(
    (({ track, version, channel }) => ({ track, version, channel }))(parseTag('tray-v0.1.31-alpha.1')),
    { track: 'tray', version: '0.1.31', channel: 'alpha' },
  );
  assert.equal(parseTag('v0.1.61').channel, 'live');
  assert.equal(parseTag('v1.2'), null);
});

test('a pre-release sorts before its release, and git order is not trusted', () => {
  // git's --sort=-version:refname puts v0.1.24-alpha.1 above v0.1.24.
  assert.ok(compareTags(parseTag('v0.1.24-alpha.1'), parseTag('v0.1.24')) < 0);
  assert.ok(compareTags(parseTag('v0.1.24-rc.1'), parseTag('v0.1.24-beta.3')) > 0);
});

test('a live release counts from the previous LIVE release on its track', () => {
  assert.equal(previousTag([...TAGS, 'v0.1.62'], 'v0.1.62'), 'v0.1.61');
  assert.equal(previousTag([...TAGS, 'tray-v0.1.31'], 'tray-v0.1.31'), 'tray-v0.1.30');
});

test('a pre-release counts from the previous tag of any kind', () => {
  assert.equal(previousTag(TAGS, 'v0.1.62-alpha.2'), 'v0.1.62-alpha.1');
  assert.equal(previousTag(TAGS, 'v0.1.62-rc.1'), 'v0.1.62-alpha.2');
  assert.equal(previousTag(TAGS, 'tray-v0.1.31-alpha.1'), 'tray-v0.1.30');
});

test('tracks never mix, and the first release has no predecessor', () => {
  assert.equal(previousTag(['tray-v0.1.30', 'v0.1.1'], 'v0.1.1'), null);
  assert.throws(() => previousTag(TAGS, 'nope'), /not a release tag/);
});
