/**
 * Tray-UI host for the unknown-line review queue.
 *
 * The queue is GROUPED by log tag. It used to list every captured shape,
 * which on a real install was 342,428 rows, almost all scoring just over
 * the old review threshold; grouped by tag the same data is a few hundred
 * groups. The Rust side (`crate::review`) splits them:
 *
 *  - **Worth a look** — tags that read like a gameplay event (a mission
 *    ending, a quantum arrival), ranked and capped. Shown open.
 *  - **Other** — engine and UI chatter. Collapsed by default, biggest
 *    first, with "ignore all" for clearing it in one go.
 *  - **Ignored** — what the user ignored, so a mistake can be undone.
 *
 * Ignoring is per GROUP and covers future captures of that tag too;
 * per-shape dismissal could never keep up with a tag that mints a new
 * variant on every line. Submitting a group sends its most frequent line,
 * and the Rust side retires the group's other variants.
 */

import { useCallback, useEffect, useState } from 'react';
import {
  api,
  type FeaturedReviewGroup,
  type IgnoredReviewGroup,
  type ReviewGroupStats,
  type UnknownLine,
} from '../api';
import { ReviewPane, type SubmitPayload } from './ReviewPane';

interface Props {
  /** Bumped by the parent when it wants the pane to refetch. */
  refreshKey?: number;
  /** Notifies the parent of the featured count, which drives the badge. */
  onCountChange?: (count: number) => void;
  /** Paired RSI handle, for the "attribute to me" label. */
  handle?: string | null;
}

export function SubmissionsPane({ refreshKey, onCountChange, handle }: Props) {
  const [featured, setFeatured] = useState<FeaturedReviewGroup[]>([]);
  const [other, setOther] = useState<ReviewGroupStats[]>([]);
  const [ignored, setIgnored] = useState<IgnoredReviewGroup[]>([]);
  const [selected, setSelected] = useState<Set<string>>(new Set());
  const [open, setOpen] = useState<string | null>(null);
  const [examples, setExamples] = useState<Record<string, UnknownLine | null>>({});
  const [error, setError] = useState<string | null>(null);
  const [loaded, setLoaded] = useState(false);

  const refresh = useCallback(async () => {
    try {
      const [groups, ign] = await Promise.all([
        api.listReviewGroups(),
        api.listIgnoredReviewGroups(),
      ]);
      setFeatured(groups.featured);
      setOther(groups.other);
      setIgnored(ign);
      setSelected(new Set());
      setError(null);
      onCountChange?.(groups.featured.length);
    } catch (e) {
      setError(String(e));
    } finally {
      setLoaded(true);
    }
  }, [onCountChange]);

  useEffect(() => {
    void refresh();
  }, [refresh, refreshKey]);

  const toggle = (tag: string) =>
    setSelected((prev) => {
      const next = new Set(prev);
      if (next.has(tag)) next.delete(tag);
      else next.add(tag);
      return next;
    });

  const ignore = async (tags: string[]) => {
    if (tags.length === 0) return;
    try {
      await api.ignoreReviewGroups(tags);
      await refresh();
    } catch (e) {
      setError(String(e));
    }
  };

  const restore = async (tag: string) => {
    try {
      await api.unignoreReviewGroup(tag);
      await refresh();
    } catch (e) {
      setError(String(e));
    }
  };

  const exampleFor = (tag: string): UnknownLine | null | undefined =>
    featured.find((g) => g.shell_tag === tag)?.example ?? examples[tag];

  const openGroup = async (tag: string) => {
    setOpen((cur) => (cur === tag ? null : tag));
    if (exampleFor(tag) !== undefined) return;
    try {
      const ex = await api.reviewGroupExample(tag);
      setExamples((m) => ({ ...m, [tag]: ex }));
    } catch (e) {
      setError(String(e));
    }
  };

  const onSubmit = useCallback(
    async (row: UnknownLine, payload: SubmitPayload) => {
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
              { before: row.context_before ?? [], after: row.context_after ?? [] },
            ],
            game_build: row.game_build ?? undefined,
            // `LogSource` is lowercase on the wire; `live` is the
            // conservative default for a row that arrived without one.
            channel: row.channel ?? 'live',
            occurrence_count: row.occurrence_count,
            // Overwritten Rust-side (`commands::submit_unknown_lines`).
            client_anon_id: '',
            attributed: payload.attributed,
          },
        ]);
        setOpen(null);
        await refresh();
      } catch (e) {
        setError(String(e));
      }
    },
    [refresh],
  );

  const renderGroup = (g: ReviewGroupStats) => {
    const ex = exampleFor(g.shell_tag);
    const isOpen = open === g.shell_tag;
    return (
      <div key={g.shell_tag} className="review-group" data-testid="review-group">
        <div className="review-group__head">
          <input
            type="checkbox"
            aria-label={`Select ${g.shell_tag}`}
            checked={selected.has(g.shell_tag)}
            onChange={() => toggle(g.shell_tag)}
          />
          <span className="review-group__tag">&lt;{g.shell_tag}&gt;</span>
          <span className="badge">×{g.occurrences.toLocaleString()}</span>
          {g.shapes > 1 ? (
            <span className="badge" title="Distinct variants of this line">
              {g.shapes.toLocaleString()} variants
            </span>
          ) : null}
          <span className="review-group__acts">
            <button type="button" onClick={() => void openGroup(g.shell_tag)}>
              {isOpen ? 'Close' : 'Review'}
            </button>
            <button type="button" onClick={() => void ignore([g.shell_tag])}>
              Ignore
            </button>
          </span>
        </div>
        {!isOpen && ex ? <pre className="raw review-group__peek">{ex.raw_line}</pre> : null}
        {isOpen && ex ? (
          <ReviewPane
            shapes={[
              {
                shape_hash: ex.shape_hash,
                raw_example: ex.raw_line,
                interest_score: ex.interest_score,
                occurrence_count: g.occurrences,
                shell_tag: ex.shell_tag,
                detected_pii: ex.detected_pii,
              },
            ]}
            onSubmit={(p) => void onSubmit(ex, p)}
            onDismiss={() => void ignore([g.shell_tag])}
            handle={handle}
          />
        ) : null}
        {isOpen && ex === null ? (
          <p className="review-group__note">This group has no open lines left.</p>
        ) : null}
      </div>
    );
  };

  const selectedCount = selected.size;
  const otherLines = other.reduce((n, g) => n + g.occurrences, 0);

  return (
    <div className="submissions-pane">
      {error && <div className="error">Error: {error}</div>}

      <div className="review-toolbar">
        <strong>Worth a look</strong>
        <span className="review-toolbar__hint">
          Lines that read like something happening in game. Submitting one
          helps a parser rule get written for it.
        </span>
        <button
          type="button"
          disabled={selectedCount === 0}
          onClick={() => void ignore([...selected])}
        >
          Ignore selected{selectedCount > 0 ? ` (${selectedCount})` : ''}
        </button>
      </div>

      {loaded && featured.length === 0 ? (
        <div className="review-pane-empty">Nothing worth a look right now.</div>
      ) : (
        featured.map(renderGroup)
      )}

      {other.length > 0 ? (
        <details className="review-other" data-testid="review-other">
          <summary>
            Other lines — {other.length.toLocaleString()} groups,{' '}
            {otherLines.toLocaleString()} lines
          </summary>
          <p className="review-group__note">
            Engine and interface chatter that rarely describes a gameplay
            event. Ignoring a group also hides its future lines.
          </p>
          <button
            type="button"
            onClick={() => {
              if (
                window.confirm(
                  `Ignore all ${other.length} other groups? You can restore them from Ignored.`,
                )
              ) {
                void ignore(other.map((g) => g.shell_tag));
              }
            }}
          >
            Ignore all other
          </button>
          {other.map(renderGroup)}
        </details>
      ) : null}

      {ignored.length > 0 ? (
        <details className="review-other" data-testid="review-ignored">
          <summary>Ignored — {ignored.length.toLocaleString()} groups</summary>
          {ignored.map((i) => (
            <div key={i.shell_tag} className="review-group__head">
              <span className="review-group__tag">&lt;{i.shell_tag}&gt;</span>
              <span className="review-group__acts">
                <button type="button" onClick={() => void restore(i.shell_tag)}>
                  Restore
                </button>
              </span>
            </div>
          ))}
        </details>
      ) : null}
    </div>
  );
}
