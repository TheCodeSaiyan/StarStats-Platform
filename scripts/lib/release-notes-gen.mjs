// Generate one release's notes: the IO half of scripts/release-notes.mjs,
// shared with scripts/update-changelog.mjs. Pure rules live in
// ./commit-rules.mjs, tag arithmetic in ./release-range.mjs.

import { execFileSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';
import { buildNotes, countSummary, loadRules } from './commit-rules.mjs';
import { parseTag, previousTag } from './release-range.mjs';

const REPO = process.env.GITHUB_REPOSITORY ?? 'TheCodeSaiyan/StarStats-Platform';
const API = process.env.STARSTATS_API_URL ?? 'https://api.starstats.app';
const RULES = fileURLToPath(new URL('../../.github/commit-rules.json', import.meta.url));

const git = (argv) => execFileSync('git', argv, { encoding: 'utf8' }).trim();

/**
 * @param {{tag: string, offline?: boolean}} opts
 * @returns {Promise<{tag: string, track: string, version: string, channel: string,
 *   from: string|null, date: string, surface: 'Tray'|'Web', summary: string, groups: any[]}>}
 */
export async function generateNotes({ tag, offline = false }) {
  const cur = parseTag(tag);
  if (!cur) throw new Error(`${tag} is not a release tag`);
  const from = previousTag(git(['tag', '--list']).split('\n'), tag);
  const range = from ? `${from}..${tag}` : tag;

  const commits = git(['log', '--no-merges', '--reverse', '--format=%H%x00%an%x00%s', range])
    .split('\n')
    .filter(Boolean)
    .map((l) => {
      const [sha, author, subject] = l.split('\0');
      return { sha, author, subject, pr: null, roadmap: null };
    });

  if (!offline) await attachPrsAndRoadmap(commits);

  const rules = loadRules(RULES);
  const surface = cur.track === 'tray' ? 'Tray' : 'Web';
  const groups = buildNotes(commits, rules, { surface });
  // The tagged commit's date, not today's: a backfilled section must carry
  // the day the release actually shipped.
  const date = git(['log', '-1', '--format=%cs', tag]);
  return {
    tag,
    track: cur.track,
    version: cur.version,
    channel: cur.channel,
    from,
    date,
    surface,
    summary: countSummary(groups),
    groups,
  };
}

async function attachPrsAndRoadmap(commits) {
  const roadmapCache = new Map();
  const roadmapItem = (slug) => {
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
      // A lookup failure costs this line its PR link and grouping, never
      // the line itself.
    }
  }
}
