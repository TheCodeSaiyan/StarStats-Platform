/**
 * Tray-UI host for the unknown-line review queue. Owns the data
 * lifecycle: polls `list_unknown_lines` on mount + on demand, adapts
 * the SQLite row shape into the `UnknownShape` the ReviewPane
 * consumes, dispatches Submit / Dismiss back through the Tauri
 * bridge, and refreshes once a write lands.
 *
 * Kept in a sibling file (not inline in App.tsx) so the
 * data-fetching logic is testable with a mocked `api.*` surface
 * without spinning the whole app shell.
 */

import { useCallback, useEffect, useMemo, useState } from 'react';
import { api, type UnknownLine } from '../api';
import { ReviewPane, type SubmitPayload, type UnknownShape } from './ReviewPane';

interface Props {
  /** Bumped by the parent when it wants the pane to refetch (e.g.
   *  after the badge polls and detects a new count). Optional —
   *  internal state changes already trigger a refetch via the
   *  Submit/Dismiss callbacks. */
  refreshKey?: number;
  /** Notifies the parent when the local cache count changes, so the
   *  badge stays in sync without an extra round-trip. */
  onCountChange?: (count: number) => void;
  /** Paired RSI handle (from the parent's already-loaded config), used
   *  to label the "attribute to me" option in the review pane. Optional
   *  — a null handle just yields a generic label. */
  handle?: string | null;
}

export function SubmissionsPane({ refreshKey, onCountChange, handle }: Props) {
  // Keep the full `UnknownLine[]` (not just the derived `UnknownShape[]`)
  // so the submit adapter can read fields the review pane intentionally
  // doesn't surface — channel, partial_structured, context_before/after,
  // game_build. Deriving shapes at render-time is cheap; refetching just
  // to recover those fields would be wasteful and racy.
  const [rows, setRows] = useState<UnknownLine[]>([]);
  const [error, setError] = useState<string | null>(null);
  // Memoized so `shapes` keeps a stable identity across renders that
  // don't touch `rows` (e.g. an `error` state change). Without this the
  // `.map` mints a fresh array every render, which would defeat the
  // downstream `useMemo` sort in ReviewPane (new prop reference → recompute).
  const shapes: UnknownShape[] = useMemo(() => rows.map(rowToShape), [rows]);

  const refresh = useCallback(async () => {
    try {
      const fresh = await api.listUnknownLines();
      setRows(fresh);
      setError(null);
      onCountChange?.(fresh.length);
    } catch (e) {
      setError(String(e));
    }
  }, [onCountChange]);

  useEffect(() => {
    void refresh();
  }, [refresh, refreshKey]);

  const onSubmit = useCallback(
    async (payload: SubmitPayload) => {
      // Find the source row so we can carry through the fields the
      // server expects but the review UI doesn't surface (channel,
      // partial_structured, context, game_build). The user only edited
      // `raw_example`, `suggested_event_name`, and `notes`.
      const row = rows.find((r) => r.shape_hash === payload.shape_hash);
      if (!row) return;
      try {
        await api.submitUnknownLines([
          {
            shape_hash: payload.shape_hash,
            raw_examples: [payload.raw_example],
            partial_structured: row.partial_structured ?? {},
            shell_tag: row.shell_tag ?? undefined,
            suggested_event_name: payload.suggested_event_name,
            notes: payload.notes,
            context_examples: [
              {
                before: row.context_before ?? [],
                after: row.context_after ?? [],
              },
            ],
            game_build: row.game_build ?? undefined,
            // `LogSource` is lowercase on the wire (`live`/`ptu`/...)
            // — `'live'` is the conservative default if a row somehow
            // arrived without a channel set.
            channel: row.channel ?? 'live',
            occurrence_count: row.occurrence_count,
            // `client_anon_id` is filled in server-side inside the
            // Tauri command (see `commands::submit_unknown_lines`).
            // The frontend value is ignored — we send an empty string
            // here purely to satisfy the TS type; the Rust path
            // overwrites it before it hits the network.
            client_anon_id: '',
            // Forced attribution choice made in the review pane. Rust
            // preserves it (`..p` spread) all the way to the HTTP body;
            // the server resolves the crediting identity when true.
            attributed: payload.attributed,
          },
        ]);
        await refresh();
      } catch (e) {
        setError(String(e));
      }
    },
    [rows, refresh]
  );

  const onDismiss = useCallback(
    async (shapeHash: string) => {
      try {
        await api.dismissUnknownLine(shapeHash);
        await refresh();
      } catch (e) {
        setError(String(e));
      }
    },
    [refresh]
  );

  return (
    <div className="submissions-pane">
      {error && <div className="error">Error: {error}</div>}
      <ReviewPane
        shapes={shapes}
        onSubmit={onSubmit}
        onDismiss={onDismiss}
        handle={handle}
      />
    </div>
  );
}

/**
 * Adapt one SQLite `UnknownLine` row into the `UnknownShape` the
 * review pane renders. We pick the most-recent raw_line off the row
 * (storage caches up to RAW_EXAMPLES_CAP) — that's `raw_line`
 * itself, which is the freshest capture per the storage upsert
 * semantics.
 */
function rowToShape(row: UnknownLine): UnknownShape {
  return {
    shape_hash: row.shape_hash,
    raw_example: row.raw_line,
    interest_score: row.interest_score,
    occurrence_count: row.occurrence_count,
    shell_tag: row.shell_tag,
    detected_pii: row.detected_pii,
  };
}
