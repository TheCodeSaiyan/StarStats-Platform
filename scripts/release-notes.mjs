#!/usr/bin/env node
// Print a release's player-facing notes, built from commit subjects.
//
//   node scripts/release-notes.mjs --tag tray-v0.1.31 [--format markdown|json] [--offline]
//
// No human or AI rewrite step: the notes are the commit subjects of
// feat/fix/perf commits on player-facing scopes, prefix stripped, with the
// glossary applied (.github/commit-rules.json). A commit whose pull request
// carries a `roadmap/<slug>` label contributes that roadmap item once, as
// its public title and summary. Tray releases list Tray notes, platform
// releases Web notes; `core` reaches both. See scripts/lib/release-notes-gen.mjs.
//
// --offline skips GitHub and roadmap lookups (no PR numbers, no grouping).

import { toMarkdown } from './lib/commit-rules.mjs';
import { generateNotes } from './lib/release-notes-gen.mjs';

function fail(code, msg) {
  console.error(`[release-notes] ${msg}`);
  process.exit(code);
}

const args = process.argv.slice(2);
const opts = { tag: null, format: 'markdown', offline: false };
for (let i = 0; i < args.length; i++) {
  const a = args[i];
  if (a === '--tag') opts.tag = args[++i];
  else if (a === '--format') opts.format = args[++i];
  else if (a === '--offline') opts.offline = true;
  else fail(2, `unknown argument ${a}`);
}
if (!opts.tag) fail(2, 'usage: release-notes.mjs --tag <tag> [--format markdown|json] [--offline]');
if (!['markdown', 'json'].includes(opts.format)) fail(2, '--format is markdown or json');

let notes;
try {
  notes = await generateNotes({ tag: opts.tag, offline: opts.offline });
} catch (e) {
  fail(2, e.message);
}

if (opts.format === 'json') {
  process.stdout.write(JSON.stringify(notes, null, 2) + '\n');
} else {
  const label = notes.track === 'tray' ? 'StarStats tray' : 'StarStats platform';
  const heading = `## ${label} ${notes.version}${notes.channel === 'live' ? '' : ` (${notes.channel})`}`;
  const body = notes.groups.length
    ? toMarkdown(notes.groups, { heading })
    : `${heading}\n\nNo player-facing changes in this release.\n`;
  process.stdout.write(body + (notes.from ? `\nChanges since ${notes.from}.\n` : ''));
}
