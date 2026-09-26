#!/usr/bin/env node
// Build a release's player-facing notes from commit subjects.
//
//   node scripts/release-notes.mjs --tag tray-v0.1.31 [--format markdown|json] [--offline]
//
// No human or AI rewrite step: the notes are the commit subjects of
// feat/fix/perf commits on player-facing scopes, prefix stripped, with
// the glossary applied (.github/commit-rules.json). A commit whose pull
// request carries a `roadmap/<slug>` label contributes that roadmap item
// once, as its public title and summary, however many commits it took.
//
// Tray releases list Tray notes only; platform releases list Web notes.
// The range is from `previousTag` (scripts/lib/release-range.mjs): the
// previous live tag for a live release, the previous tag of any kind for
// a pre-release.
//
// --offline skips GitHub and roadmap lookups (no PR numbers, no roadmap
// grouping). Used by tests and when the network is unavailable.

import { execFileSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';
import { buildNotes, countSummary, loadRules, toMarkdown } from './lib/commit-rules.mjs';
import { parseTag, previousTag } from './lib/release-range.mjs';

const REPO = process.env.GITHUB_REPOSITORY ?? 'TheCodeSaiyan/StarStats-Platform';
const API = process.env.STARSTATS_API_URL ?? 'https://api.starstats.app';
const RULES = fileURLToPath(new URL('../.github/commit-rules.json', import.meta.url));

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

const cur = parseTag(opts.tag);
if (!cur) fail(2, `${opts.tag} is not a release tag`);

const git = (argv) => execFileSync('git', argv, { encoding: 'utf8' }).trim();
const from = previousTag(git(['tag', '--list']).split('\n'), opts.tag);
const range = from ? `${from}..${opts.tag}` : opts.tag;

const commits = git(['log', '--no-merges', '--reverse', '--format=%H%x00%an%x00%s', range])
  .split('\n')
  .filter(Boolean)
  .map((l) => {
    const [sha, author, subject] = l.split('\0');
    return { sha, author, subject, pr: null, roadmap: null };
  });

if (!opts.offline) {
  const roadmapCache = new Map();
  const roadmapItem = async (slug) => {
    if (!roadmapCache.has(slug)) {
      roadmapCache.set(
        slug,
        fetch(`${API}/v1/roadmap/${encodeURIComponent(slug)}`)
          .then((r) => (r.ok ? r.json() : null))
          .then((j) => (j && j.public ? { slug, title: j.title, summary: j.summary ?? '' } : null))
          .catch(() => null),
      );
    }
    return roadmapCache.get(slug);
  };
  for (const c of commits) {
    try {
      const prs = JSON.parse(
        execFileSync('gh', ['api', `repos/${REPO}/commits/${c.sha}/pulls`], { encoding: 'utf8' }),
      );
      const pr = prs.find((p) => p.merged_at) ?? prs[0];
      if (!pr) continue;
      c.pr = pr.number;
      const label = (pr.labels ?? []).map((l) => l.name).find((n) => n.startsWith('roadmap/'));
      if (label) c.roadmap = await roadmapItem(label.slice('roadmap/'.length));
    } catch {
      // A lookup failure costs the PR link and grouping for this line,
      // never the line itself.
    }
  }
}

const rules = loadRules(RULES);
const surface = cur.track === 'tray' ? 'Tray' : 'Web';
const groups = buildNotes(commits, rules, { surface });
const label = cur.track === 'tray' ? 'StarStats tray' : 'StarStats platform';

if (opts.format === 'json') {
  process.stdout.write(
    JSON.stringify(
      {
        track: cur.track,
        tag: opts.tag,
        version: cur.version,
        channel: cur.channel,
        from,
        summary: countSummary(groups),
        groups,
      },
      null,
      2,
    ) + '\n',
  );
} else {
  const heading = `## ${label} ${cur.version}${cur.channel === 'live' ? '' : ` (${cur.channel})`}`;
  const body = groups.length
    ? toMarkdown(groups, { heading })
    : `${heading}\n\nNo player-facing changes in this release.\n`;
  process.stdout.write(body + (from ? `\nChanges since ${from}.\n` : ''));
}
