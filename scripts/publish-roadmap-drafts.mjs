#!/usr/bin/env node
// Bulk-publish auto-drafted roadmap changelog entries, optionally
// fleshing out title + body first so the tray card actually names the
// parent roadmap item.
//
// Companion to `roadmap-emit-event.mjs`. The emitter pushes shipped-CI
// events into `/v1/internal/roadmap/events`; on a first-time Shipped
// transition the receiver auto-drafts a changelog entry (Phase 7, spec
// §8.4). The auto-draft body is terse — channel + commit SHA, nothing
// referencing the parent item by name. This script fetches the public
// roadmap (id → {slug, title}), rewrites each draft to lead with the
// parent item title + slug, then publishes it.
//
// Default behavior (no flags):
//   1. List all pending drafts.
//   2. For each draft, look up its parent item via roadmap_item_id and
//      compose a richer title + body that names the item and its slug.
//   3. PATCH-style edit via POST /v1/admin/roadmap/changelog/{id}/edit.
//   4. Publish via POST /v1/admin/roadmap/changelog/{id}/publish.
//
// Read-only safety: every mutation is non-destructive. No draft row is
// deleted, no item is archived, no project board state is touched. The
// script's mutations are `published_at NULL → now()` and (optionally)
// `title/body` rewrite on the same draft row.
//
// Required env vars:
//   - STARSTATS_API_URL      Base URL of starstats-server, e.g.
//                            `https://api.starstats.app`. No trailing /.
//   - STARSTATS_ADMIN_TOKEN  Bearer JWT for an account with the `admin`
//                            staff role (see staff_roles.rs +
//                            STARSTATS_BOOTSTRAP_ADMIN_HANDLES).
//
// Flags:
//   --dry-run              List drafts AND preview the proposed
//                          title/body rewrite for each. No POSTs.
//   --no-edit              Skip the title/body rewrite step; publish
//                          drafts with their auto-generated content.
//   --slug <name>          Exact-match the parent item slug (case-
//                          sensitive). E.g. `--slug SS` publishes only
//                          drafts whose parent roadmap item has the
//                          slug `SS` (matches the value of the
//                          ROADMAP_ITEM_SLUG GitHub Actions variable).
//                          Combine with --filter for AND-style scoping.
//   --filter <substring>   Case-insensitive substring match against
//                          either the draft title OR the parent item
//                          title/slug. Only matching drafts are touched.
//   --limit <N>            Stop after publishing N drafts.
//   --yes                  Skip the interactive confirmation prompt.
//                          Required for non-TTY runs (CI, scripts).
//   --ok-if-empty          Exit 0 (instead of 1) when --slug matches
//                          zero drafts. Use in CI where the same
//                          channel re-shipping legitimately means
//                          there's no new draft to publish.
//   --help                 Print usage and exit 0.
//
// Exit codes:
//   0 — every targeted draft published OK (or --dry-run completed).
//   1 — at least one edit or publish call returned non-2xx. The script
//       keeps going after a single failure and reports a summary.
//   2 — config error (missing env, bad flag, invalid URL).
//
// Example:
//   STARSTATS_API_URL=https://api.starstats.app \
//   STARSTATS_ADMIN_TOKEN=eyJ... \
//   node scripts/publish-roadmap-drafts.mjs --dry-run

import { stdin as input, stdout as output } from 'node:process';
import readline from 'node:readline/promises';

import {
  composeRewrite,
  formatDraftLine,
} from './lib/publish-drafts-lib.mjs';

function fatal(code, msg) {
  console.error(`[publish-drafts] ${msg}`);
  process.exit(code);
}

function usage() {
  console.log(
    [
      'Usage: node scripts/publish-roadmap-drafts.mjs [--dry-run] [--no-edit] [--slug <name>] [--filter <substr>] [--limit <N>] [--yes] [--ok-if-empty]',
      '',
      'Env: STARSTATS_API_URL, STARSTATS_ADMIN_TOKEN',
      '',
      'Fleshes out the title/body of each pending roadmap changelog draft',
      '(naming its parent item + slug) and publishes it. Publish is',
      'non-destructive — flips published_at NULL → now().',
    ].join('\n'),
  );
}

// ---------- arg parsing ----------------------------------------------------

const args = process.argv.slice(2);
const opts = {
  dryRun: false,
  noEdit: false,
  slug: null,
  filter: null,
  limit: null,
  yes: false,
  okIfEmpty: false,
};

for (let i = 0; i < args.length; i++) {
  const a = args[i];
  if (a === '--help' || a === '-h') {
    usage();
    process.exit(0);
  } else if (a === '--dry-run') {
    opts.dryRun = true;
  } else if (a === '--no-edit') {
    opts.noEdit = true;
  } else if (a === '--yes' || a === '-y') {
    opts.yes = true;
  } else if (a === '--ok-if-empty') {
    // For CI: a release that re-ships the same channel doesn't create
    // a new draft (events.rs:310 — first-time transition only), so
    // there's legitimately nothing to publish. Exit 0 in that case
    // instead of fataling on the --slug zero-match check.
    opts.okIfEmpty = true;
  } else if (a === '--slug') {
    const v = args[++i];
    if (!v) fatal(2, '--slug requires a value');
    opts.slug = v;
  } else if (a === '--filter') {
    const v = args[++i];
    if (!v) fatal(2, '--filter requires a value');
    opts.filter = v.toLowerCase();
  } else if (a === '--limit') {
    const v = args[++i];
    const n = Number.parseInt(v, 10);
    if (!Number.isInteger(n) || n <= 0) fatal(2, `--limit requires a positive integer (got ${v})`);
    opts.limit = n;
  } else {
    fatal(2, `unknown flag: ${a} (try --help)`);
  }
}

// ---------- env validation -------------------------------------------------

const apiUrl = (process.env.STARSTATS_API_URL || '').replace(/\/+$/, '');
const token = process.env.STARSTATS_ADMIN_TOKEN || '';
if (!apiUrl) fatal(2, 'STARSTATS_API_URL not set');
if (!token) fatal(2, 'STARSTATS_ADMIN_TOKEN not set');

let parsedUrl;
try {
  parsedUrl = new URL(apiUrl);
} catch {
  fatal(2, `STARSTATS_API_URL is not a valid URL: ${apiUrl}`);
}
if (
  parsedUrl.protocol !== 'https:' &&
  parsedUrl.hostname !== 'localhost' &&
  parsedUrl.hostname !== '127.0.0.1'
) {
  fatal(2, `STARSTATS_API_URL must be https:// (or localhost): ${apiUrl}`);
}

// ---------- helpers --------------------------------------------------------

const authHeaders = {
  Authorization: `Bearer ${token}`,
  Accept: 'application/json',
};

async function listDrafts() {
  const resp = await fetch(`${apiUrl}/v1/admin/roadmap/changelog/drafts`, {
    method: 'GET',
    headers: authHeaders,
  });
  if (!resp.ok) {
    const text = await resp.text();
    if (resp.status === 401) fatal(2, `401 unauthorized — STARSTATS_ADMIN_TOKEN rejected: ${text.slice(0, 200)}`);
    if (resp.status === 403)
      fatal(
        2,
        `403 forbidden — token's account lacks the admin staff role (check STARSTATS_BOOTSTRAP_ADMIN_HANDLES)`,
      );
    fatal(1, `list_drafts ${resp.status}: ${text.slice(0, 500)}`);
  }
  const body = await resp.json();
  if (!body || !Array.isArray(body.drafts)) {
    fatal(1, `list_drafts: response missing drafts[] (got ${JSON.stringify(body).slice(0, 200)})`);
  }
  return body.drafts;
}

// Walk the public roadmap to build an id → { slug, title } index. The
// admin list-items endpoint may not exist; the public list is enough
// because the tray only renders public items anyway (a draft tied to a
// non-public item couldn't appear in /whats-new regardless of how
// nicely its title reads).
async function fetchItemIndex() {
  const resp = await fetch(`${apiUrl}/v1/roadmap`, {
    method: 'GET',
    headers: { Accept: 'application/json' },
  });
  if (!resp.ok) {
    const text = await resp.text();
    fatal(1, `list_roadmap ${resp.status}: ${text.slice(0, 200)}`);
  }
  const body = await resp.json();
  const items = Array.isArray(body?.items) ? body.items : [];
  const index = new Map();
  for (const item of items) {
    if (item && item.id) {
      index.set(item.id, { slug: item.slug || '', title: item.title || '(untitled item)' });
    }
  }
  return index;
}

async function editDraft(id, payload) {
  const resp = await fetch(`${apiUrl}/v1/admin/roadmap/changelog/${id}/edit`, {
    method: 'POST',
    headers: { ...authHeaders, 'Content-Type': 'application/json' },
    body: JSON.stringify(payload),
  });
  const text = await resp.text();
  if (resp.ok) return { ok: true };
  if (resp.status === 404) return { ok: false, soft: true, reason: 'not_found_or_already_published' };
  if (resp.status === 400) return { ok: false, reason: `400 bad request: ${text.slice(0, 200)}` };
  return { ok: false, reason: `${resp.status} ${text.slice(0, 200)}` };
}

async function publishOne(id) {
  const resp = await fetch(`${apiUrl}/v1/admin/roadmap/changelog/${id}/publish`, {
    method: 'POST',
    headers: authHeaders,
  });
  const text = await resp.text();
  if (resp.ok) return { ok: true };
  if (resp.status === 404) return { ok: false, soft: true, reason: 'not_found_or_already_published' };
  return { ok: false, reason: `${resp.status} ${text.slice(0, 200)}` };
}

function printRewritePreview(draft, rewrite) {
  console.log(`    ── ${draft.id} ──`);
  console.log(`    before  title: ${draft.title || '(empty)'}`);
  console.log(`    after   title: ${rewrite.title}`);
  console.log(`    after   body:`);
  for (const line of rewrite.body.split('\n')) console.log(`        ${line}`);
}

async function confirm(msg) {
  if (opts.yes) return true;
  if (!input.isTTY) fatal(2, 'non-TTY run requires --yes to confirm the publish batch');
  const rl = readline.createInterface({ input, output });
  try {
    const ans = (await rl.question(`${msg} [y/N] `)).trim().toLowerCase();
    return ans === 'y' || ans === 'yes';
  } finally {
    rl.close();
  }
}

// ---------- main -----------------------------------------------------------

const [drafts, itemIndex] = await Promise.all([listDrafts(), fetchItemIndex()]);

if (drafts.length === 0) {
  console.log('[publish-drafts] no pending drafts. Nothing to do.');
  process.exit(0);
}

let targets = drafts;
if (opts.slug) {
  targets = targets.filter((d) => itemIndex.get(d.roadmap_item_id)?.slug === opts.slug);
  if (targets.length === 0) {
    if (opts.okIfEmpty) {
      console.log(`[publish-drafts] --slug "${opts.slug}" matched zero drafts; --ok-if-empty set, exiting 0.`);
      process.exit(0);
    }
    // The slug must exist in the public roadmap index for the
    // filter to match. If it doesn't, the user has either mistyped
    // the slug, or the parent item isn't Public=Yes — flag that
    // explicitly so they don't think the script silently no-op'd.
    const known = Array.from(itemIndex.values()).map((v) => v.slug).filter(Boolean);
    fatal(
      1,
      `--slug "${opts.slug}" matched zero drafts. Known public slugs: ${known.length === 0 ? '(none)' : known.join(', ')}`,
    );
  }
}
if (opts.filter) {
  targets = targets.filter((d) => {
    const parent = itemIndex.get(d.roadmap_item_id);
    const hay = `${d.title || ''} ${parent?.title || ''} ${parent?.slug || ''}`.toLowerCase();
    return hay.includes(opts.filter);
  });
}
if (opts.limit !== null) {
  targets = targets.slice(0, opts.limit);
}

const scopeBits = [];
if (opts.slug) scopeBits.push(`slug=${opts.slug}`);
if (opts.filter) scopeBits.push(`filter="${opts.filter}"`);
if (opts.limit !== null) scopeBits.push(`limit=${opts.limit}`);
const scopeNote = scopeBits.length === 0 ? '' : ` (${targets.length} match ${scopeBits.join(', ')})`;
console.log(`[publish-drafts] ${drafts.length} pending draft(s) total${scopeNote}:`);
for (const d of targets) console.log(formatDraftLine(d, itemIndex));

if (targets.length === 0) {
  console.log('[publish-drafts] no drafts match the filter. Exiting.');
  process.exit(0);
}

// Build the rewrites up front so dry-run can preview them and the
// publish loop can reuse them.
const rewrites = new Map();
if (!opts.noEdit) {
  for (const d of targets) rewrites.set(d.id, composeRewrite(d, itemIndex));
}

if (opts.dryRun) {
  if (!opts.noEdit) {
    console.log('[publish-drafts] --dry-run: proposed rewrites:');
    for (const d of targets) printRewritePreview(d, rewrites.get(d.id));
  }
  console.log(`[publish-drafts] --dry-run: would ${opts.noEdit ? 'publish' : 'edit + publish'} ${targets.length} draft(s).`);
  process.exit(0);
}

const action = opts.noEdit ? `Publish ${targets.length} draft(s) as-is` : `Edit + publish ${targets.length} draft(s)`;
if (!(await confirm(`${action}?`))) {
  console.log('[publish-drafts] aborted by user.');
  process.exit(0);
}

let edited = 0;
let published = 0;
let softSkipped = 0;
const failures = [];

for (const d of targets) {
  if (!opts.noEdit) {
    const rewrite = rewrites.get(d.id);
    const editResult = await editDraft(d.id, rewrite);
    if (!editResult.ok) {
      if (editResult.soft) {
        softSkipped += 1;
        console.log(`  skip    ${d.id}  edit: ${editResult.reason}`);
        continue;
      }
      failures.push({ id: d.id, phase: 'edit', reason: editResult.reason });
      console.log(`  FAIL    ${d.id}  edit: ${editResult.reason}`);
      continue;
    }
    edited += 1;
  }

  const pubResult = await publishOne(d.id);
  if (pubResult.ok) {
    published += 1;
    const label = rewrites.get(d.id)?.title || d.title || '(untitled)';
    console.log(`  ok      ${d.id}  ${label}`);
  } else if (pubResult.soft) {
    softSkipped += 1;
    console.log(`  skip    ${d.id}  publish: ${pubResult.reason}`);
  } else {
    failures.push({ id: d.id, phase: 'publish', reason: pubResult.reason });
    console.log(`  FAIL    ${d.id}  publish: ${pubResult.reason}`);
  }
}

console.log(
  `[publish-drafts] summary: edited=${edited} published=${published} skipped=${softSkipped} failed=${failures.length}`,
);
process.exit(failures.length === 0 ? 0 : 1);
