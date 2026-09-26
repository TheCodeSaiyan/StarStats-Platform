// CHANGELOG.md's generated block.
//
// Everything between the two markers is owned by scripts/update-changelog.mjs
// and rewritten from commit subjects; everything outside is left exactly as
// it is (the intro, and the hand-written 1.x history from before the
// repository was re-initialised). One section per live release, keyed by its
// heading, so re-running for a release replaces its section instead of
// duplicating it.

import { toMarkdown } from './commit-rules.mjs';

export const START = '<!-- generated:start (scripts/update-changelog.mjs; do not edit by hand) -->';
export const END = '<!-- generated:end -->';

const LEGACY_NOTE =
  'Entries below this line predate the August 2026 repository reset, when versions restarted at 0.1.x. They are kept as written.';

/** "## [Tray 0.1.30] - 2026-09-21" */
export function sectionHeading(notes) {
  const name = notes.track === 'tray' ? 'Tray' : 'Platform';
  return `## [${name} ${notes.version}] - ${notes.date}`;
}

export function sectionBody(notes) {
  if (notes.groups.length === 0) return 'No player-facing changes.\n';
  // toMarkdown's `### New` sits one level below the `## [...]` heading.
  return toMarkdown(notes.groups);
}

/** Split a CHANGELOG into the text around the block and its sections. */
export function parseChangelog(md) {
  const s = md.indexOf(START);
  const e = md.indexOf(END);
  if (s === -1 || e === -1 || e < s) {
    return { before: null, sections: new Map(), after: md };
  }
  const inner = md.slice(s + START.length, e);
  const sections = new Map();
  const parts = inner.split(/^(?=## \[)/m);
  for (const part of parts) {
    const m = /^(## \[[^\n]+)\n([\s\S]*)$/.exec(part.trim() + '\n');
    if (m && m[1].startsWith('## [')) sections.set(m[1], m[2].trim() + '\n');
  }
  return { before: md.slice(0, s), sections, after: md.slice(e + END.length) };
}

const dateOf = (heading) => (/(\d{4}-\d{2}-\d{2})$/.exec(heading) ?? [, ''])[1];
const trackOf = (heading) => (/^## \[(\w+)/.exec(heading) ?? [, ''])[1];
const versionOf = (heading) => (/ (\d+\.\d+\.\d+)\]/.exec(heading) ?? [, '0.0.0'])[1];
// Several releases can ship on one day, so the version, not the text,
// orders them: "0.1.59" < "0.1.61" but "0.1.9" > "0.1.10" as strings.
const compareVersions = (a, b) => {
  const x = a.split('.').map(Number);
  const y = b.split('.').map(Number);
  return x[0] - y[0] || x[1] - y[1] || x[2] - y[2];
};

/**
 * Put release sections into the changelog, replacing any with the same
 * heading. Newest date first; on the same date Platform before Tray.
 */
export function upsertSections(md, entries) {
  let { before, sections, after } = parseChangelog(md);
  if (before === null) {
    // First run: the block takes the place of the empty [Unreleased] stub.
    const stub = /^## \[Unreleased\]\n+- \(nothing yet\)\n+/m;
    const at = stub.exec(md);
    if (at) {
      before = md.slice(0, at.index);
      after = `\n${LEGACY_NOTE}\n\n` + md.slice(at.index + at[0].length);
    } else {
      const firstSection = md.search(/^## /m);
      before = firstSection === -1 ? md + '\n' : md.slice(0, firstSection);
      after = firstSection === -1 ? '' : `\n${LEGACY_NOTE}\n\n` + md.slice(firstSection);
    }
  }
  for (const { heading, body } of entries) sections.set(heading, body);
  const ordered = [...sections.entries()].sort(
    ([a], [b]) =>
      dateOf(b).localeCompare(dateOf(a)) ||
      trackOf(a).localeCompare(trackOf(b)) ||
      compareVersions(versionOf(b), versionOf(a)),
  );
  const block = ordered.map(([h, b]) => `${h}\n\n${b.trim()}\n`).join('\n');
  // Exactly one blank line after the block, however many runs came before.
  return `${before}${START}\n\n${block}\n${END}\n\n${after.replace(/^\n+/, '')}`;
}
