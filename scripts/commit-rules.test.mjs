// Tests for scripts/lib/commit-rules.mjs against the real rules file.
//
//   node --test scripts/commit-rules.test.mjs

import { test } from 'node:test';
import assert from 'node:assert/strict';
import { fileURLToPath } from 'node:url';

import {
  buildNotes,
  checkSubject,
  countSummary,
  loadRules,
  parseSubject,
  toMarkdown,
  toPlayerText,
} from './lib/commit-rules.mjs';

const rules = loadRules(fileURLToPath(new URL('../.github/commit-rules.json', import.meta.url)));
const ok = (s, author = 'Someone') => assert.deepEqual(checkSubject(s, author, rules), [], s);
const bad = (s, re, author = 'Someone') => {
  const p = checkSubject(s, author, rules);
  assert.ok(p.some((x) => re.test(x)), `${s}\n  got: ${JSON.stringify(p)}`);
};

test('well-formed subjects from the real history pass', () => {
  ok('fix(web): count the orders, not the page they arrived on');
  ok('feat(server): answer the friend commerce URL the web has always called');
  ok('chore: bump platform to v0.1.61-alpha.1');
  ok('ci(dependabot): group the codeql action bumps');
  ok('feat(tray): add a Friends tab and desktop notifications');
});

test('automation is exempt by author or by prefix', () => {
  ok('release-manifests: tray-live → tray-v0.1.30');
  ok('merge live manifest into next (from tray-v0.1.30)');
  ok('Anything at all', 'github-actions[bot]');
  ok('Bump the npm group across 1 directory', 'dependabot[bot]');
});

test('shape, type, scope, case, length and full stop are enforced', () => {
  bad('Friends tab', /type\(scope\): what changed/);
  bad('feature(tray): add friends', /Unknown type "feature"/);
  bad('feat(friends): add friends', /Unknown scope "friends"/);
  bad('fix(tray): Keep the list in order', /lower case/);
  ok('fix(tray): RSI handle copies again');
  bad('fix(tray): keep the list in order.', /full stop/);
  bad(`fix(tray): ${'x'.repeat(80)}`, /limit is 72/);
});

test('player-facing subjects may not use jargon; internal ones may', () => {
  bad('feat(tray): friends pane, notification toasts, friends in PII redaction', /pii, toast, pane|may not use/);
  bad('fix(web): the admin console list loads', /admin console/);
  bad('feat(tray): show toasts for new releases', /toast/);
  bad('fix(tray): endpoints respond again', /endpoint/);
  ok('refactor(server): split the notification store from the handler');
  ok('fix(server): retry the spicedb schema read');
});

test('parseSubject reads type, scope and breaking marker', () => {
  assert.deepEqual(parseSubject('feat(web)!: drop the old share link'), {
    type: 'feat',
    scope: 'web',
    breaking: true,
    text: 'drop the old share link',
  });
  assert.equal(parseSubject('not conventional'), null);
});

test('glossary turns engineering words into player words', () => {
  assert.equal(toPlayerText('show notification toasts in the review pane', rules), 'Show desktop notifications in the review tab');
});

test('notes: only player-facing commits, grouped, one line per roadmap item', () => {
  const friends = { slug: 'friends-notifications', title: 'Friends & notifications', summary: 'Add friends and get notified.' };
  const commits = [
    { sha: 'a', subject: 'feat(server): friends, blocks and mutes', pr: 134, roadmap: friends },
    { sha: 'b', subject: 'feat(web): add friends and friend requests', pr: 134, roadmap: friends },
    { sha: 'c', subject: 'feat(tray): add a Friends tab', pr: 134, roadmap: friends },
    { sha: 'd', subject: 'fix(tray): group the Review tab by line type', pr: 134 },
    { sha: 'e', subject: 'perf(web): load the dashboard in one round trip', pr: 140 },
    { sha: 'f', subject: 'ci: cache the rust toolchain' },
    { sha: 'g', subject: 'release-manifests: tray-live → tray-v0.1.31', author: 'github-actions[bot]' },
  ];
  const groups = buildNotes(commits, rules);
  assert.deepEqual(groups.map((g) => g.kind), ['New', 'Improved', 'Fixed']);
  const newLines = groups[0].lines;
  assert.equal(newLines.length, 1, 'three commits on one roadmap item make one line');
  assert.equal(newLines[0].text, 'Friends & notifications: Add friends and get notified');
  assert.deepEqual(newLines[0].surfaces, ['Web', 'Tray'], 'the server commit is not player-facing');
  assert.equal(groups[2].lines[0].text, 'Group the Review tab by line type');
  assert.equal(countSummary(groups), '1 new, 1 improved, 1 fixed');

  const tray = buildNotes(commits, rules, { surface: 'Tray' });
  assert.deepEqual(tray.map((g) => g.kind), ['New', 'Fixed'], 'web-only perf left out of the tray');
});

test('a fix inside a roadmap pull request keeps its own line', () => {
  const item = { slug: 'friends-notifications', title: 'Friends & notifications', summary: 'Add friends.' };
  const groups = buildNotes(
    [
      { sha: 'a', subject: 'feat(tray): add a Friends tab', pr: 134, roadmap: item },
      { sha: 'b', subject: 'fix(tray): group the Review tab by line type', pr: 134, roadmap: item },
    ],
    rules,
  );
  assert.deepEqual(
    groups.map((g) => [g.kind, g.lines.map((l) => l.text)]),
    [
      ['New', ['Friends & notifications: Add friends']],
      ['Fixed', ['Group the Review tab by line type']],
    ],
  );
});

test('a core fix reaches both the tray and the platform notes', () => {
  const commits = [{ sha: 'a', subject: 'fix(core): read mission starts again' }];
  assert.deepEqual(buildNotes(commits, rules)[0].lines[0].surfaces, ['Tray', 'Web']);
  assert.equal(buildNotes(commits, rules, { surface: 'Tray' }).length, 1);
  assert.equal(buildNotes(commits, rules, { surface: 'Web' }).length, 1);
});

test('markdown for GitHub and CHANGELOG.md', () => {
  const groups = buildNotes(
    [{ sha: 'a', subject: 'fix(tray): keep the review list in order', pr: 7 }],
    rules,
  );
  assert.equal(
    toMarkdown(groups, { heading: '## [Tray 0.1.31] - 2026-09-26', withSurfaces: true }),
    '## [Tray 0.1.31] - 2026-09-26\n\n### Fixed\n- Keep the review list in order. (Tray) #7\n',
  );
});
