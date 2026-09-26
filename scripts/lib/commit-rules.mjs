// Commit-subject rules and the release-note mapping built on them.
//
// Pure functions over `.github/commit-rules.json`, shared by:
//   - scripts/check-commits.mjs  (the commit-msg hook and the CI check)
//   - scripts/release-notes.mjs  (release notes, GitHub bodies, CHANGELOG.md)
//
// Release notes are generated from commit subjects with no human or AI
// rewrite step. That only works if the subjects are held to rules, and
// commits can come from people or tools that do not know them, so the
// rules are enforced here rather than written down and hoped for.

import { readFileSync } from 'node:fs';

const HEADER = /^(?<type>[a-z]+)(?:\((?<scope>[a-z0-9-]+)\))?(?<bang>!)?: (?<text>.+)$/;

export function loadRules(path) {
  return JSON.parse(readFileSync(path, 'utf8'));
}

/** Parse a Conventional Commits header. `null` when it is not one. */
export function parseSubject(subject) {
  const m = HEADER.exec(String(subject ?? '').trim());
  if (!m) return null;
  return {
    type: m.groups.type,
    scope: m.groups.scope ?? null,
    breaking: Boolean(m.groups.bang),
    text: m.groups.text,
  };
}

/** True when the commit is automation the rules do not apply to. */
export function isExempt(subject, author, rules) {
  const ex = rules.exempt ?? {};
  if ((ex.authors ?? []).includes(author)) return true;
  return (ex.subject_prefixes ?? []).some((p) => String(subject).startsWith(p));
}

/** Words from the jargon list in `text`: whole word, any case, plural too. */
export function jargonIn(text, rules) {
  const lc = ` ${String(text).toLowerCase().replace(/[^a-z0-9 ]+/g, ' ')} `;
  return (rules.jargon ?? []).filter((w) => {
    const word = w.toLowerCase();
    return lc.includes(` ${word} `) || lc.includes(` ${word}s `) || lc.includes(` ${word}es `);
  });
}

/**
 * Check one commit subject. Returns a list of problems, each a sentence
 * the author can act on; empty when the subject passes.
 */
export function checkSubject(subject, author, rules) {
  const s = String(subject ?? '').replace(/\r?\n[\s\S]*$/, '');
  if (isExempt(s, author, rules)) return [];
  const problems = [];
  const max = rules.subject?.max_length ?? 72;
  if (s.length > max) {
    problems.push(`Subject is ${s.length} characters; the limit is ${max}.`);
  }
  const parsed = parseSubject(s);
  if (!parsed) {
    problems.push(
      'Subject must look like "type(scope): what changed", e.g. "fix(tray): keep the review list in order".',
    );
    return problems;
  }
  if (!rules.types.includes(parsed.type)) {
    problems.push(`Unknown type "${parsed.type}". Allowed: ${rules.types.join(', ')}.`);
  }
  if (parsed.scope && !rules.scopes.includes(parsed.scope)) {
    problems.push(
      `Unknown scope "${parsed.scope}". Allowed: ${rules.scopes.join(', ')}. Add a new scope to .github/commit-rules.json in the same change if it is genuinely new.`,
    );
  }
  if (rules.subject?.lowercase_after_prefix && /^[A-Z]/.test(parsed.text) && !/^[A-Z]{2,}/.test(parsed.text)) {
    problems.push('Start the description in lower case (an acronym such as "RSI" is fine).');
  }
  if (rules.subject?.no_trailing_period && /\.$/.test(parsed.text)) {
    problems.push('Drop the full stop at the end of the subject.');
  }
  if (isPlayerFacing(parsed, rules)) {
    const found = jargonIn(parsed.text, rules);
    if (found.length > 0) {
      problems.push(
        `This subject becomes a release note players read, so it may not use: ${found.join(', ')}. Say what the player can now do or no longer runs into.`,
      );
    }
  }
  return problems;
}

/** A feat/fix/perf on a scope players can see. */
export function isPlayerFacing(parsed, rules) {
  return Boolean(
    parsed &&
      rules.player_types?.[parsed.type] &&
      parsed.scope &&
      rules.player_scopes?.[parsed.scope],
  );
}

/** Engineering words to player words, longest phrase first, then a capital. */
export function toPlayerText(text, rules) {
  let out = String(text);
  const entries = Object.entries(rules.glossary ?? {}).sort((a, b) => b[0].length - a[0].length);
  for (const [from, to] of entries) {
    out = out.replace(new RegExp(`\\b${from.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')}\\b`, 'gi'), to);
  }
  return out.charAt(0).toUpperCase() + out.slice(1);
}

const KIND_ORDER = ['New', 'Improved', 'Fixed'];

/**
 * Build release notes from commits.
 *
 * @param {Array<{sha: string, subject: string, author?: string, pr?: number|null,
 *   roadmap?: {slug: string, title: string, summary?: string}|null}>} commits
 * @param {object} rules
 * @param {{surface?: 'Tray'|'Web'|null}} [opts] Limit to one surface.
 * @returns {{kind: string, lines: Array<{text: string, surfaces: string[], prs: number[], roadmap: string|null}>}[]}
 *
 * A commit linked to a roadmap item contributes that item ONCE, as its
 * title and summary, however many commits it took. Everything else
 * contributes its subject. Lines are de-duplicated by text.
 */
export function buildNotes(commits, rules, opts = {}) {
  const byKind = new Map(KIND_ORDER.map((k) => [k, new Map()]));
  for (const c of commits) {
    if (isExempt(c.subject, c.author ?? '', rules)) continue;
    const parsed = parseSubject(c.subject);
    if (!isPlayerFacing(parsed, rules)) continue;
    // A scope may map to several surfaces: `core` is the parser both the
    // tray and the server run, so its fixes reach both.
    const mapped = [rules.player_scopes[parsed.scope]].flat();
    if (opts.surface && !mapped.includes(opts.surface)) continue;
    const surfaces = opts.surface ? [opts.surface] : mapped;
    const kind = rules.player_types[parsed.type];
    // A roadmap item describes the FEATURE, so it stands in only for new
    // work. A fix or improvement that shipped in the same pull request is
    // a different thing to tell the player, and keeps its own subject;
    // otherwise the feature's line appeared under Fixed as well as New.
    const roadmap = kind === 'New' ? c.roadmap : null;
    const key = roadmap ? `roadmap:${roadmap.slug}` : `text:${parsed.text.toLowerCase()}`;
    const text = roadmap
      ? `${roadmap.title}${roadmap.summary ? `: ${roadmap.summary.replace(/\.$/, '')}` : ''}`
      : toPlayerText(parsed.text, rules);
    const bucket = byKind.get(kind);
    const line = bucket.get(key) ?? {
      text,
      surfaces: [],
      prs: [],
      roadmap: roadmap?.slug ?? null,
    };
    for (const s of surfaces) if (!line.surfaces.includes(s)) line.surfaces.push(s);
    if (c.pr && !line.prs.includes(c.pr)) line.prs.push(c.pr);
    bucket.set(key, line);
  }
  return KIND_ORDER.map((kind) => ({ kind, lines: [...byKind.get(kind).values()] })).filter(
    (g) => g.lines.length > 0,
  );
}

/** "5 new, 1 improved, 1 fixed" for the tray toast. */
export function countSummary(groups) {
  return groups.map((g) => `${g.lines.length} ${g.kind.toLowerCase()}`).join(', ');
}

/** Markdown for a GitHub Release body or a CHANGELOG.md section. */
export function toMarkdown(groups, { heading = null, withPrs = true, withSurfaces = false } = {}) {
  const out = [];
  if (heading) out.push(heading, '');
  for (const g of groups) {
    out.push(`### ${g.kind}`);
    for (const l of g.lines) {
      const tags = withSurfaces ? ` (${l.surfaces.join(', ')})` : '';
      const prs = withPrs && l.prs.length ? ` ${l.prs.map((n) => `#${n}`).join(' ')}` : '';
      out.push(`- ${l.text.replace(/\.$/, '')}.${tags}${prs}`);
    }
    out.push('');
  }
  return out.join('\n').trimEnd() + '\n';
}
