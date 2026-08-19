// Tests for the pure rewrite-composition helpers used by
// scripts/publish-roadmap-drafts.mjs. Run with:
//
//   node --test scripts/publish-roadmap-drafts.test.mjs
//
// No external test framework — uses node's built-in test runner so the
// script directory stays dependency-free (same convention as
// roadmap-emit-event.mjs).

import { test } from 'node:test';
import assert from 'node:assert/strict';

import {
  shortSha,
  channelLabel,
  composeRewrite,
  formatDraftLine,
} from './lib/publish-drafts-lib.mjs';

// ---------- shortSha -------------------------------------------------------

test('shortSha truncates 40-char SHAs to 7', () => {
  assert.equal(shortSha('a3f8b2e1c4d5f6a7b8c9d0e1f2a3b4c5d6e7f8a9'), 'a3f8b2e');
});

test('shortSha leaves short identifiers untouched', () => {
  assert.equal(shortSha('abc'), 'abc');
});

test('shortSha returns null for null/undefined input', () => {
  assert.equal(shortSha(null), null);
  assert.equal(shortSha(undefined), null);
});

test('shortSha returns null for empty string', () => {
  assert.equal(shortSha(''), null);
});

// ---------- channelLabel ---------------------------------------------------

test('channelLabel maps known channels to title-case labels', () => {
  assert.equal(channelLabel('live'), 'Live');
  assert.equal(channelLabel('beta'), 'Beta');
  assert.equal(channelLabel('alpha'), 'Alpha');
  assert.equal(channelLabel('tech-preview'), 'Tech Preview');
});

test('channelLabel passes unknown channels through verbatim', () => {
  assert.equal(channelLabel('canary'), 'canary');
});

test('channelLabel returns "?" for empty input', () => {
  assert.equal(channelLabel(''), '?');
  assert.equal(channelLabel(null), '?');
});

// ---------- composeRewrite -------------------------------------------------

function draftFixture(over = {}) {
  return {
    id: 'd1',
    roadmap_item_id: 'item-uuid',
    channel: 'live',
    title: 'Shipped to live',
    body: 'auto-draft body line\nci_run: https://example/runs/1',
    previous_shipped_sha: 'a3f8b2e1c4d5f6a7b8c9d0e1f2a3b4c5d6e7f8a9',
    shipped_sha: 'c81d4f0e1c4d5f6a7b8c9d0e1f2a3b4c5d6e7f8a',
    created_at: '2026-05-27T10:30:00Z',
    published_at: null,
    published_by: null,
    ...over,
  };
}

function indexFixture(slug = 'SS', title = 'StarStats') {
  return new Map([['item-uuid', { slug, title }]]);
}

test('composeRewrite leads title with parent item + channel', () => {
  const out = composeRewrite(draftFixture(), indexFixture());
  assert.equal(out.title, 'StarStats — Shipped to Live');
});

test('composeRewrite body names the item and shows the slug', () => {
  const out = composeRewrite(draftFixture(), indexFixture());
  assert.match(out.body, /\*\*StarStats\*\* is now available on the Live channel\./);
  assert.match(out.body, /Track this item on the roadmap: `SS`/);
});

test('composeRewrite shows commit range when both SHAs present', () => {
  const out = composeRewrite(draftFixture(), indexFixture());
  assert.match(out.body, /Build range: `a3f8b2e` → `c81d4f0`/);
});

test('composeRewrite shows single Build when previous_shipped_sha missing', () => {
  const out = composeRewrite(draftFixture({ previous_shipped_sha: null }), indexFixture());
  assert.match(out.body, /Build: `c81d4f0`/);
  assert.doesNotMatch(out.body, /Build range/);
});

test('composeRewrite preserves original body below a divider', () => {
  const out = composeRewrite(draftFixture(), indexFixture());
  assert.match(out.body, /---\nauto-draft body line\nci_run: https:\/\/example\/runs\/1/);
});

test('composeRewrite omits divider when original body is empty', () => {
  const out = composeRewrite(draftFixture({ body: '   ' }), indexFixture());
  assert.doesNotMatch(out.body, /---/);
});

test('composeRewrite falls back to short-uuid framing when parent not in index', () => {
  const out = composeRewrite(draftFixture({ roadmap_item_id: 'abcdef12-3456-7890-abcd-ef1234567890' }), new Map());
  assert.equal(out.title, 'item abcdef12 — Shipped to Live');
  // No slug line when the parent isn't found — keeps the rewrite honest.
  assert.doesNotMatch(out.body, /Track this item on the roadmap/);
});

test('composeRewrite uses channelLabel mapping (tech-preview)', () => {
  const out = composeRewrite(draftFixture({ channel: 'tech-preview' }), indexFixture());
  assert.equal(out.title, 'StarStats — Shipped to Tech Preview');
  assert.match(out.body, /Tech Preview channel/);
});

// ---------- formatDraftLine ------------------------------------------------

test('formatDraftLine includes timestamp, channel, id, and slug+title', () => {
  const line = formatDraftLine(draftFixture(), indexFixture());
  assert.match(line, /2026-05-27 10:30/);
  assert.match(line, /live\s+d1\s+\[SS\] StarStats/);
});

test('formatDraftLine flags drafts whose parent is not in the public index', () => {
  const line = formatDraftLine(draftFixture(), new Map());
  assert.match(line, /\(not in public list\)/);
});

test('formatDraftLine tolerates a missing created_at', () => {
  const line = formatDraftLine(draftFixture({ created_at: null }), indexFixture());
  assert.match(line, /\?\?\?\?-\?\?-\?\? \?\?:\?\?/);
});
