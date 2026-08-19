/**
 * ReactNode mirror of `format.ts`'s `prettifySummary` /
 * `humanTitleForEntry`. The legacy string helpers replace class-name
 * tokens inline by swapping in `display_name` from a flat lookup —
 * that turns `AEGS_Avenger_Stalker` into `Aegis Avenger Stalker` in
 * the summary text but produces a plain string with no navigation
 * affordance.
 *
 * The React versions parse the same tokens out of the rendered
 * summary and wrap each one in a `<TrayEntityLink>` so the user can
 * click through to the KB page in the web app via the shell plugin.
 * For tokens that aren't in any catalogue, the legacy fallback
 * applies — the raw string remains as plain text.
 *
 * Co-existing with the legacy string helpers rather than replacing
 * them: clipboard / share / OG-card paths still want a flat string.
 */

import type { ReactNode } from 'react';
import {
  type AllReferenceBundles,
  REFERENCE_CATEGORIES,
  type ReferenceCategory,
  type ReferenceEntry,
  findEntityInBundles,
} from '../../lib/reference';
import { TrayEntityLink } from '../kb/TrayEntityLink';
import { humanizeEventType, EVENT_VERB_TABLE } from './format';

/// Tokens shorter than this don't get the display-name scan — too
/// many false-positives against ordinary English (e.g. "of", "a"
/// landing in titles). Underscore-bearing class_names always go
/// through the regex path regardless of length.
const MIN_DISPLAY_NAME_LENGTH = 3;

/// English words that occur frequently in event summaries but also
/// appear (verbatim) as wiki entries. Skipping the display-name
/// scan for these keeps "Joined PU shard..." from getting a
/// spurious link on the word "PU" if a wiki entry happens to share
/// that name.
const DISPLAY_NAME_STOPWORDS = new Set<string>([
  'the',
  'and',
  'for',
  'pu',
  'fps',
  'eva',
  'tdd',
  'cig',
  'rsi',
]);

interface EntityHit {
  start: number;
  end: number;
  category: ReferenceCategory;
  entry: ReferenceEntry;
  /// The exact substring from the raw summary — preserves casing
  /// so the rendered text still matches what the server wrote.
  rawText: string;
}

/// Find all catalogue display_name occurrences in `raw` (word-
/// bounded, case-insensitive). Skips entries shorter than
/// `MIN_DISPLAY_NAME_LENGTH` and any lowercased name in
/// `DISPLAY_NAME_STOPWORDS` to keep ordinary English from
/// false-matching.
function findDisplayNameHits(
  raw: string,
  bundles: AllReferenceBundles,
): EntityHit[] {
  const lowered = raw.toLowerCase();
  const hits: EntityHit[] = [];
  const wordChar = /[a-z0-9_]/i;
  for (const category of REFERENCE_CATEGORIES) {
    for (const entry of bundles[category].list) {
      const name = entry.display_name;
      if (name.length < MIN_DISPLAY_NAME_LENGTH) continue;
      const nameLower = name.toLowerCase();
      if (DISPLAY_NAME_STOPWORDS.has(nameLower)) continue;
      let from = 0;
      while (true) {
        const idx = lowered.indexOf(nameLower, from);
        if (idx === -1) break;
        const before = idx > 0 ? lowered[idx - 1] : '';
        const after =
          idx + nameLower.length < lowered.length
            ? lowered[idx + nameLower.length]
            : '';
        const wordBefore = before !== '' && wordChar.test(before);
        const wordAfter = after !== '' && wordChar.test(after);
        if (!wordBefore && !wordAfter) {
          hits.push({
            start: idx,
            end: idx + name.length,
            category,
            entry,
            rawText: raw.slice(idx, idx + name.length),
          });
        }
        from = idx + nameLower.length;
      }
    }
  }
  return hits;
}

/// Find all class_name regex matches that resolve in any catalogue.
/// Mirrors the legacy `prettifySummary` regex — uppercase-led
/// identifier with at least one underscore.
function findClassNameHits(
  raw: string,
  bundles: AllReferenceBundles,
): EntityHit[] {
  const hits: EntityHit[] = [];
  for (const match of raw.matchAll(/[A-Z][A-Z0-9]*_[A-Za-z0-9_]+/g)) {
    const token = match[0];
    const start = match.index ?? 0;
    const found = findEntityInBundles(token, bundles);
    if (found) {
      hits.push({
        start,
        end: start + token.length,
        category: found.category,
        entry: found.entry,
        rawText: token,
      });
    }
  }
  return hits;
}

/// Merge the two hit lists into a non-overlapping sequence ordered
/// by position. When two hits overlap, the longer one wins (so
/// "Stanton 2b" beats a bare "Stanton" inside it). Equal-length
/// overlaps resolve in favour of the class_name match — the more
/// specific identifier — by inserting class_name hits first.
function dedupeAndOrderHits(
  classHits: EntityHit[],
  nameHits: EntityHit[],
): EntityHit[] {
  const all = [...classHits, ...nameHits].sort((a, b) => {
    if (a.start !== b.start) return a.start - b.start;
    return b.end - b.start - (a.end - a.start); // longer first at same start
  });
  const accepted: EntityHit[] = [];
  let cursor = 0;
  for (const hit of all) {
    if (hit.start < cursor) continue;
    accepted.push(hit);
    cursor = hit.end;
  }
  return accepted;
}

/**
 * Replace catalogue-known tokens in a Rust-formatted summary string
 * with `<TrayEntityLink>` instances that open the corresponding KB
 * page when clicked.
 *
 * Two passes feed the matcher:
 *   1. Class-name regex (`AEGS_Avenger_Stalker`, `QNTM_FuelCell`) —
 *      same pattern as the legacy `prettifySummary` string helper.
 *   2. Display-name scan — every catalogue entry's `display_name`
 *      (>= 3 chars, not in the stopword set) is searched as a
 *      word-bounded substring of `raw`. Catches single-word
 *      ("Stanton") and multi-word ("New Babbage", "Brio's Breaker
 *      Yard") friendly names the regex would miss.
 *
 * Overlapping hits resolve in favour of the longer match, so a
 * full "Stanton 2b" wins over a bare "Stanton" embedded in it.
 *
 * Returns the original string unchanged when `bundles` or
 * `webOrigin` is missing — surfaces that can't navigate also get
 * the legacy plain-text rendering, no broken affordances.
 */
export function prettifySummaryReact(
  raw: string,
  bundles: AllReferenceBundles | undefined,
  webOrigin: string | null | undefined,
): ReactNode {
  if (!raw) return raw;
  if (!bundles || !webOrigin) return raw;
  const hits = dedupeAndOrderHits(
    findClassNameHits(raw, bundles),
    findDisplayNameHits(raw, bundles),
  );
  if (hits.length === 0) return raw;
  const parts: ReactNode[] = [];
  let cursor = 0;
  let nodeKey = 0;
  for (const hit of hits) {
    if (hit.start > cursor) parts.push(raw.slice(cursor, hit.start));
    parts.push(
      <TrayEntityLink
        key={`ent-${nodeKey++}`}
        category={hit.category}
        classKey={hit.entry.class_name}
        catalog={bundles[hit.category].catalog}
        label={hit.rawText}
        webOrigin={webOrigin}
      />,
    );
    cursor = hit.end;
  }
  if (cursor < raw.length) parts.push(raw.slice(cursor));
  return <>{parts}</>;
}

/**
 * ReactNode-returning twin of `humanTitleForEntry`. Picks the best
 * player-facing title for a TimelineEntry-shaped row and surfaces
 * any embedded catalogue identifiers as `<TrayEntityLink>` instances.
 *
 * When `bundles` or `webOrigin` is missing, behaves identically to
 * `humanTitleForEntry` — plain string output, no clickable affordances.
 */
export function humanTitleForEntryReact(
  entry: { event_type: string; summary: string },
  bundles: AllReferenceBundles | undefined,
  webOrigin: string | null | undefined,
): ReactNode {
  const trimmed = entry.summary.trim();
  // Rust-side unparseable-payload fallback shape: route around it so
  // the headline doesn't surface the raw snake_case key. Mirror
  // `humanTitleForEntry` exactly.
  const isFallback = trimmed.startsWith(`${entry.event_type} (unparseable`);
  if (trimmed && !isFallback) {
    return prettifySummaryReact(trimmed, bundles, webOrigin);
  }
  return EVENT_VERB_TABLE[entry.event_type] ?? humanizeEventType(entry.event_type);
}
