#!/usr/bin/env node
// Write live releases into CHANGELOG.md's generated block.
//
//   node scripts/update-changelog.mjs --tag tray-v0.1.31 [--tag v0.1.62] [--offline]
//   node scripts/update-changelog.mjs --backfill 10 [--offline]
//
// --backfill N writes the N most recent LIVE releases on each track, for
// the first run and after a gap. A section is keyed by its heading, so
// running again for the same release replaces it; the file outside the
// generated markers is never touched. Pre-release tags are refused:
// CHANGELOG.md records what reached live players.

import { execFileSync } from 'node:child_process';
import { readFileSync, writeFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { sectionBody, sectionHeading, upsertSections } from './lib/changelog.mjs';
import { generateNotes } from './lib/release-notes-gen.mjs';
import { compareTags, parseTag } from './lib/release-range.mjs';

const FILE = fileURLToPath(new URL('../CHANGELOG.md', import.meta.url));

function fail(code, msg) {
  console.error(`[changelog] ${msg}`);
  process.exit(code);
}

const args = process.argv.slice(2);
const tags = [];
let backfill = 0;
let offline = false;
for (let i = 0; i < args.length; i++) {
  if (args[i] === '--tag') tags.push(args[++i]);
  else if (args[i] === '--backfill') backfill = Number.parseInt(args[++i], 10);
  else if (args[i] === '--offline') offline = true;
  else fail(2, `unknown argument ${args[i]}`);
}
if (tags.length === 0 && !(backfill > 0)) {
  fail(2, 'usage: update-changelog.mjs --tag <live tag>... | --backfill <N> [--offline]');
}

if (backfill > 0) {
  const live = execFileSync('git', ['tag', '--list'], { encoding: 'utf8' })
    .split('\n')
    .map(parseTag)
    .filter((t) => t && t.channel === 'live');
  for (const track of ['platform', 'tray']) {
    live
      .filter((t) => t.track === track)
      .sort(compareTags)
      .slice(-backfill)
      .forEach((t) => tags.push(t.tag));
  }
}

const entries = [];
for (const tag of tags) {
  const t = parseTag(tag);
  if (!t) fail(2, `${tag} is not a release tag`);
  if (t.channel !== 'live') fail(2, `${tag} is a pre-release; CHANGELOG.md records live releases only`);
  const notes = await generateNotes({ tag, offline });
  entries.push({ heading: sectionHeading(notes), body: sectionBody(notes) });
  console.log(`[changelog] ${sectionHeading(notes).slice(3)}: ${notes.summary || 'nothing player-facing'}`);
}

writeFileSync(FILE, upsertSections(readFileSync(FILE, 'utf8'), entries));
console.log(`[changelog] wrote ${entries.length} section(s) to CHANGELOG.md`);
