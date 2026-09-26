// node --test scripts/changelog.test.mjs

import { test } from 'node:test';
import assert from 'node:assert/strict';
import { END, START, parseChangelog, sectionBody, sectionHeading, upsertSections } from './lib/changelog.mjs';

const ORIGINAL = `# Changelog

Intro text.

## [Unreleased]

- (nothing yet)

## [1.8.1] - 2026-05-22

Old hand-written notes.
`;

const tray = {
  track: 'tray',
  version: '0.1.30',
  date: '2026-09-21',
  groups: [{ kind: 'Fixed', lines: [{ text: 'Read mission starts', surfaces: ['Tray'], prs: [125] }] }],
};
const platform = { track: 'platform', version: '0.1.61', date: '2026-09-21', groups: [] };

const entry = (n) => ({ heading: sectionHeading(n), body: sectionBody(n) });

test('headings name the track, version and ship date', () => {
  assert.equal(sectionHeading(tray), '## [Tray 0.1.30] - 2026-09-21');
  assert.equal(sectionHeading(platform), '## [Platform 0.1.61] - 2026-09-21');
});

test('first run replaces the empty Unreleased stub and keeps the old history', () => {
  const out = upsertSections(ORIGINAL, [entry(tray)]);
  assert.ok(out.startsWith('# Changelog\n\nIntro text.\n\n' + START));
  assert.ok(!out.includes('(nothing yet)'));
  assert.ok(out.includes('### Fixed\n- Read mission starts. #125'));
  assert.ok(out.includes('predate the August 2026 repository reset'));
  assert.ok(out.includes('## [1.8.1] - 2026-05-22\n\nOld hand-written notes.'));
  assert.ok(out.indexOf(END) < out.indexOf('## [1.8.1]'));
});

test('re-running replaces a section instead of duplicating it', () => {
  const once = upsertSections(ORIGINAL, [entry(tray)]);
  const twice = upsertSections(once, [entry(tray), entry(platform)]);
  assert.equal(twice.split('## [Tray 0.1.30]').length, 2, 'one Tray 0.1.30 section');
  assert.equal(parseChangelog(twice).sections.size, 2);
  assert.equal(upsertSections(twice, [entry(tray), entry(platform)]), twice, 'idempotent');
});

test('newest first; Platform before Tray on the same day', () => {
  const older = { ...tray, version: '0.1.29', date: '2026-09-17' };
  const out = upsertSections(ORIGINAL, [entry(older), entry(tray), entry(platform)]);
  const order = [...parseChangelog(out).sections.keys()];
  assert.deepEqual(order, [
    '## [Platform 0.1.61] - 2026-09-21',
    '## [Tray 0.1.30] - 2026-09-21',
    '## [Tray 0.1.29] - 2026-09-17',
  ]);
});

test('same-day releases order by version, not text', () => {
  const v = (version) => entry({ ...platform, version, date: '2026-09-21' });
  const out = upsertSections(ORIGINAL, [v('0.1.9'), v('0.1.61'), v('0.1.10')]);
  const versions = [...parseChangelog(out).sections.keys()].map((h) => /(\d+\.\d+\.\d+)\]/.exec(h)[1]);
  assert.deepEqual(versions, ['0.1.61', '0.1.10', '0.1.9']);
});

test('a release with nothing player-facing says so', () => {
  assert.equal(sectionBody(platform), 'No player-facing changes.\n');
});
