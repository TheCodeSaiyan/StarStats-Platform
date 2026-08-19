/**
 * Pure sort helper for Knowledge-base category listings. Kept out of
 * the (server-only) page module so it can be unit-tested in isolation.
 *
 * The caller supplies the primary-key extractor, so this stays
 * decoupled from the per-category `Summary` shape — the page passes
 * `display_name` for the "name" sort or the facet value (manufacturer
 * / classification) for the facet sort. A `display_name` tiebreak
 * keeps ordering stable when two entries share the primary key.
 */

export type SortDir = 'asc' | 'desc';

export function sortKbEntries<T extends { display_name: string }>(
  entries: readonly T[],
  primaryValue: (e: T) => string,
  dir: SortDir,
): T[] {
  return [...entries].sort((a, b) => {
    const primary = primaryValue(a).localeCompare(primaryValue(b));
    const cmp =
      primary !== 0 ? primary : a.display_name.localeCompare(b.display_name);
    return dir === 'desc' ? -cmp : cmp;
  });
}

/** Coerce a raw `?dir=` param to a valid direction (defaults asc). */
export function parseSortDir(raw: string | undefined): SortDir {
  return raw === 'desc' ? 'desc' : 'asc';
}
