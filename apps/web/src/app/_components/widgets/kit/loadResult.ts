/**
 * Telling a widget that FAILED from one that is merely EMPTY.
 *
 * `load` returned `null` for both, and every render path treated that as "no
 * data". So a query that timed out looked exactly like an account with
 * nothing in it — and `elements.tsx` already says what that costs:
 *
 *   AN ELEMENT THAT VANISHES IS INDISTINGUISHABLE FROM A BROKEN ONE. […]
 *   That ambiguity cost real time on the loadout pane — the tray had stopped
 *   producing the events it reads, and the surface said nothing at all.
 *
 * It costs more than time now. `/me` fans out ~37 API calls in one unbounded
 * `Promise.allSettled` against a 16-connection pool with a 5s acquire timeout
 * (`main.rs:181`), behind a 15s client abort (`api.ts:175`). Past some row
 * count the aggregates slow down, the burst can no longer be served, and
 * requests start failing — which rendered as "nothing recorded yet" to users
 * holding hundreds of thousands of records. Reported as data being "wiped and
 * then eventually loading back in", which is precisely what a transient
 * failure looks like when the UI has no word for failure.
 *
 * `null` still means EMPTY, so a widget that deliberately returns null for a
 * genuinely empty window is unchanged. Failure is its own value.
 */

/** A load that threw or was caught. NOT an empty result. */
export const LOAD_FAILED = Symbol.for('starstats.widget.loadFailed');

export type LoadFailed = typeof LOAD_FAILED;

/** What a widget's `load` may resolve to: data, empty (`null`), or failure. */
export type LoadResult<D> = D | null | LoadFailed;

export function isLoadFailure(v: unknown): v is LoadFailed {
  return v === LOAD_FAILED;
}

/**
 * True when there is nothing to draw — either outcome.
 *
 * Callers that genuinely do not care (a gate deciding whether to build a body
 * at all) use this; anything that RENDERS must branch on `isLoadFailure`
 * instead, or it reintroduces the ambiguity this module exists to remove.
 */
export function hasNoData(v: unknown): boolean {
  return v == null || isLoadFailure(v);
}
