/**
 * Small fuzzy matcher for pick-lists (ship search, cohort search).
 *
 * A query is split into tokens and every token has to land somewhere in
 * the candidate, but "somewhere" is generous: a word prefix (`glad` →
 * Gladius), a substring (`enger` → Avenger), a one-edit typo against a
 * word (`gladuis` → Gladius, `stalkr` → Stalker) or an in-order
 * subsequence of the whole string (`avstk` → **Av**enger **St**al**k**er).
 * Each way of landing scores differently so the obvious hit sorts first.
 *
 * Deliberately dependency-free: the lists it serves are a few hundred
 * names at most, ranked on every keystroke in the browser.
 */

/** Lowercase, and fold everything that is not a letter or digit into a
 *  single space so `Rod's Fuel 'N Supplies` and `rods fuel n supplies`
 *  compare equal. */
export function normalizeForMatch(s: string): string {
  return s
    .toLowerCase()
    .replace(/[^\p{L}\p{N}]+/gu, ' ')
    .trim();
}

/** Optimal-string-alignment distance: Levenshtein plus adjacent
 *  transposition as one edit, so a swapped pair (`ui` for `iu`) costs 1. */
export function editDistance(a: string, b: string): number {
  const rows = a.length + 1;
  const cols = b.length + 1;
  const d: number[][] = Array.from({ length: rows }, () => new Array<number>(cols).fill(0));
  for (let i = 0; i < rows; i++) d[i][0] = i;
  for (let j = 0; j < cols; j++) d[0][j] = j;
  for (let i = 1; i < rows; i++) {
    for (let j = 1; j < cols; j++) {
      const cost = a[i - 1] === b[j - 1] ? 0 : 1;
      d[i][j] = Math.min(d[i - 1][j] + 1, d[i][j - 1] + 1, d[i - 1][j - 1] + cost);
      if (i > 1 && j > 1 && a[i - 1] === b[j - 2] && a[i - 2] === b[j - 1]) {
        d[i][j] = Math.min(d[i][j], d[i - 2][j - 2] + 1);
      }
    }
  }
  return d[a.length][b.length];
}

/** Index just past the last character of `token` matched in order inside
 *  `text` starting at `from`, or -1 when the token is not a subsequence. */
function subsequenceEnd(token: string, text: string, from: number): number {
  let i = from;
  for (const ch of token) {
    i = text.indexOf(ch, i);
    if (i < 0) return -1;
    i += 1;
  }
  return i;
}

function scoreToken(token: string, text: string, words: readonly string[]): number | null {
  let best: number | null = null;
  const consider = (s: number) => {
    if (best === null || s > best) best = s;
  };
  words.forEach((w, idx) => {
    const firstWordBonus = idx === 0 ? 15 : 0;
    if (w === token) consider(120 + firstWordBonus);
    else if (w.startsWith(token)) consider(100 + firstWordBonus);
    // A one-edit typo only counts on tokens long enough that a single
    // edit is unlikely to be a different word entirely.
    else if (token.length >= 4 && w.length >= 4 && editDistance(token, w) <= 1) consider(55);
  });
  if (text.includes(token)) consider(70);
  if (best === null) {
    // In-order subsequence; tighter spans score higher. Search from every
    // occurrence of the first character so `st` in "Avenger Stalker"
    // prefers the "St" of Stalker over the spread-out "s…t".
    let start = text.indexOf(token[0]);
    while (start >= 0) {
      const end = subsequenceEnd(token, text, start);
      if (end < 0) break;
      const spread = end - start - token.length;
      consider(35 - Math.min(spread, 25));
      start = text.indexOf(token[0], start + 1);
    }
  }
  return best;
}

/** Score `text` against `query`, or `null` when some query token finds
 *  no home in it. Higher is better. */
export function fuzzyScore(query: string, text: string): number | null {
  const q = normalizeForMatch(query);
  if (!q) return null;
  const t = normalizeForMatch(text);
  if (!t) return null;
  const words = t.split(' ');
  let total = 0;
  for (const token of q.split(' ')) {
    const s = scoreToken(token, t, words);
    if (s === null) return null;
    total += s;
  }
  // The whole query appearing contiguously beats the same tokens
  // scattered across the name.
  if (q.includes(' ') && t.includes(q)) total += 30;
  return total;
}

/** Items that match `query`, best first. Ties go to the shorter label,
 *  then alphabetical, so `Arrow` sorts ahead of `Arrow Something`. */
export function rankFuzzy<T>(
  query: string,
  items: readonly T[],
  label: (item: T) => string,
  limit = Number.POSITIVE_INFINITY,
): T[] {
  const scored: Array<{ item: T; score: number; text: string }> = [];
  for (const item of items) {
    const text = label(item);
    const score = fuzzyScore(query, text);
    if (score !== null) scored.push({ item, score, text });
  }
  scored.sort(
    (a, b) =>
      b.score - a.score ||
      a.text.length - b.text.length ||
      a.text.localeCompare(b.text),
  );
  return scored.slice(0, limit).map((s) => s.item);
}
