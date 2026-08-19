/**
 * Contract-run outcome text + badge styling — pure functions, no React,
 * so `/me/contracts` and its tests can exercise the state → text mapping
 * in isolation.
 *
 * `closed_by` is the field that matters, not `state`: it is what tells an
 * OBSERVED HUD outcome (`hud_complete`/`hud_failed`/`hud_withdrawn`) apart
 * from a stream-ended INFERENCE (`session_end`/`game_crash`/`session_gap`/
 * `shard_change`) — that distinction is the entire point of this page.
 * `state` only breaks the tie for `closed_by === 'none'` runs, to tell
 * "still running" apart from "the stream ended with no closing evidence
 * at all". See `crate::repo::closed_by_str` for the canonical string set.
 */

/** `.ss-badge--*` suffix; `''` renders the neutral, unsuffixed badge. */
export type BadgeVariant = '' | 'ok' | 'warn' | 'danger';

/** Human outcome text for a run. Never renders a raw `closed_by`/`state`
 *  enum value — every branch is spelled out, including the fallback. */
export function outcomeText(state: string, closedBy: string): string {
  switch (closedBy) {
    case 'hud_complete':
      return 'completed';
    case 'hud_failed':
      return 'failed';
    case 'hud_withdrawn':
      return 'withdrawn';
    case 'session_end':
      return 'abandoned — app exit';
    case 'game_crash':
      return 'abandoned — game crash';
    case 'session_gap':
      return 'abandoned — session gap';
    case 'shard_change':
      return 'abandoned — changed server';
    case 'superseded':
      return 'superseded by a later accept';
    default:
      // closed_by === 'none' (or anything unrecognized, defensively).
      return state === 'in_progress' ? 'still in progress' : 'no outcome recorded';
  }
}

/** Badge colour for a run's outcome — observed failures/completions get a
 *  definite colour, inferred abandonment gets `warn` (it's a best guess,
 *  not eyewitness), everything else (withdrawn/superseded/in-progress/
 *  unknown) stays neutral. */
export function outcomeBadgeVariant(closedBy: string): BadgeVariant {
  switch (closedBy) {
    case 'hud_complete':
      return 'ok';
    case 'hud_failed':
      return 'danger';
    case 'session_end':
    case 'game_crash':
    case 'session_gap':
    case 'shard_change':
      return 'warn';
    default:
      return '';
  }
}

/** Human label for one `ContractStepRow.state` (`in_progress` | `complete`
 *  | `withdrawn` | `failed` — see `step_state_str`). Like `outcomeText`,
 *  never surfaces a raw enum: an unrecognized state is de-snaked rather
 *  than passed through, so a future `StepState` variant reads as English
 *  in the UI instead of leaking `some_new_state`. */
export function stepStateLabel(state: string): string {
  switch (state) {
    case 'complete':
      return 'complete';
    case 'withdrawn':
      return 'withdrawn';
    case 'failed':
      return 'failed';
    case 'in_progress':
      return 'in progress';
    default:
      return state.replace(/_/g, ' ');
  }
}

/** Marker colour token name for a step's state (`''` = dim/neutral). */
export function stepBadgeVariant(state: string): BadgeVariant {
  switch (state) {
    case 'complete':
      return 'ok';
    case 'failed':
      return 'danger';
    case 'withdrawn':
      return 'warn';
    default:
      return '';
  }
}

/** `accepted_at` → `closed_at` in seconds (for `fmtDuration`), or `null`
 *  when either bound is missing, unparsable, or inverted — omitted from
 *  the UI rather than shown as a bogus "0m" or negative span. */
export function runDurationSecs(
  acceptedAt: string | null | undefined,
  closedAt: string | null | undefined,
): number | null {
  if (!acceptedAt || !closedAt) return null;
  const start = Date.parse(acceptedAt);
  const end = Date.parse(closedAt);
  if (Number.isNaN(start) || Number.isNaN(end) || end < start) return null;
  return (end - start) / 1000;
}

/** Newest-first by `accepted_at`; runs with no `accepted_at` sort last.
 *  Mirrors the server's own `accepted_at DESC` order (see `repo.rs`'s
 *  `contract_runs` query) but is re-applied here defensively rather than
 *  trusted outright — a future change to that ORDER BY would otherwise
 *  silently reorder this page with no client-side signal. */
export function byAcceptedDesc(
  a: { accepted_at?: string | null },
  b: { accepted_at?: string | null },
): number {
  const ta = a.accepted_at ? Date.parse(a.accepted_at) : -Infinity;
  const tb = b.accepted_at ? Date.parse(b.accepted_at) : -Infinity;
  return tb - ta;
}
