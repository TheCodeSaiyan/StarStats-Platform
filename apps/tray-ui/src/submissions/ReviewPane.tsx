/**
 * Side panel listing unknown-shape candidates from the local SQLite
 * cache. Each row exposes the shape header (hash, occurrence count,
 * interest score, optional shell tag), the most-recent raw example,
 * PII toggles per detected token, free-text "suggested event name"
 * + "notes", and Submit / Dismiss buttons.
 *
 * Submit applies the chosen redactions to the raw example before
 * handing the payload to the parent's `onSubmit`; Dismiss bubbles the
 * shape hash up so the parent can mark it dismissed in storage.
 *
 * Rows sort by `interest_score * occurrence_count` desc — most
 * actionable shapes float to the top.
 */

import { useMemo, useState } from 'react';
import { PiiToggle, type PiiToken } from './PiiToggle';

export interface UnknownShape {
  shape_hash: string;
  raw_example: string;
  interest_score: number;
  occurrence_count: number;
  shell_tag?: string | null;
  detected_pii: PiiToken[];
}

export interface SubmitPayload {
  shape_hash: string;
  /** raw example with the user's chosen redactions applied. */
  raw_example: string;
  suggested_event_name?: string;
  notes?: string;
  /** keyed by `${kind}@${start}` — preserves the per-token decision
   *  so the caller can include it in the submission record. */
  redactions: Record<string, boolean>;
  /** The user's forced attribution choice for this shape. `true`
   *  credits the paired account; `false` posts anonymously. Never
   *  undefined once Submit fires — the button stays disabled until
   *  the user picks one. */
  attributed: boolean;
}

interface Props {
  shapes: UnknownShape[];
  onSubmit: (payload: SubmitPayload) => void;
  onDismiss: (shapeHash: string) => void;
  /** Paired RSI handle, when known, used to label the "attribute to
   *  me" option (e.g. "Attribute to @Daisy"). Null/undefined falls
   *  back to a generic label — attribution still works, it's the
   *  server-side identity that credits the account. */
  handle?: string | null;
}

export function ReviewPane({ shapes, onSubmit, onDismiss, handle }: Props) {
  // Memoized so the whole list isn't copied and re-sorted on every
  // parent render (e.g. each keystroke in a row's notes field, which
  // re-renders this pane). Recomputes only when `shapes` changes.
  const sorted = useMemo(
    () =>
      [...shapes].sort(
        (a, b) =>
          b.interest_score * b.occurrence_count -
          a.interest_score * a.occurrence_count
      ),
    [shapes]
  );
  if (sorted.length === 0) {
    return <div className="review-pane-empty">No unknown lines to review.</div>;
  }
  return (
    <div className="review-pane">
      {sorted.map((s) => (
        <ShapeRow
          key={s.shape_hash}
          shape={s}
          onSubmit={onSubmit}
          onDismiss={onDismiss}
          handle={handle}
        />
      ))}
    </div>
  );
}

function ShapeRow({
  shape,
  onSubmit,
  onDismiss,
  handle,
}: {
  shape: UnknownShape;
  onSubmit: (p: SubmitPayload) => void;
  onDismiss: (s: string) => void;
  handle?: string | null;
}) {
  const [redactions, setRedactions] = useState<Record<string, boolean>>(() =>
    Object.fromEntries(
      shape.detected_pii.map((t) => [tokenKey(t), t.default_redact])
    )
  );
  const [suggestedName, setSuggestedName] = useState('');
  const [notes, setNotes] = useState('');
  // Forced attribution choice — `null` until the user explicitly
  // picks. Submit stays disabled while null so there is no silent
  // default: the user must consciously choose attributed vs anonymous
  // before a shape leaves their machine.
  const [attributed, setAttributed] = useState<boolean | null>(null);

  const submit = () => {
    // Guard: `submit` is only reachable when a choice is made (button
    // is disabled otherwise), but narrow the type for the payload.
    if (attributed === null) return;
    onSubmit({
      shape_hash: shape.shape_hash,
      raw_example: applyRedactions(shape.raw_example, shape.detected_pii, redactions),
      suggested_event_name: suggestedName || undefined,
      notes: notes || undefined,
      redactions,
      attributed,
    });
  };

  // Group name must be unique per row so selecting a choice on one
  // shape doesn't clear the radios on another.
  const attrGroup = `attribution-${shape.shape_hash}`;
  const attributedLabel = handle
    ? `Attribute to @${handle}`
    : 'Attribute to my account';

  return (
    <div className="shape-row" data-testid="shape-row">
      <header>
        <code>{shape.shape_hash}</code>
        <span className="badge">×{shape.occurrence_count}</span>
        <span className="badge interest">{shape.interest_score}</span>
        {shape.shell_tag && (
          <span className="badge tag">&lt;{shape.shell_tag}&gt;</span>
        )}
      </header>
      <pre className="raw">{shape.raw_example}</pre>
      {shape.detected_pii.length > 0 && (
        <div className="pii-toggles">
          {shape.detected_pii.map((t) => (
            <PiiToggle
              key={tokenKey(t)}
              token={t}
              onChange={(redact) =>
                setRedactions((r) => ({ ...r, [tokenKey(t)]: redact }))
              }
            />
          ))}
        </div>
      )}
      <input
        type="text"
        placeholder="Suggested event name (optional)"
        value={suggestedName}
        onChange={(e) => setSuggestedName(e.target.value)}
      />
      <textarea
        placeholder="Notes for the rule author (optional)"
        value={notes}
        onChange={(e) => setNotes(e.target.value)}
      />
      <fieldset className="attribution">
        <legend>How should this be credited?</legend>
        <label>
          <input
            type="radio"
            name={attrGroup}
            checked={attributed === true}
            onChange={() => setAttributed(true)}
          />
          {attributedLabel}
        </label>
        <label>
          <input
            type="radio"
            name={attrGroup}
            checked={attributed === false}
            onChange={() => setAttributed(false)}
          />
          Submit anonymously (shown as @community)
        </label>
      </fieldset>
      <div className="actions">
        <button type="button" onClick={submit} disabled={attributed === null}>
          Submit
        </button>
        <button type="button" onClick={() => onDismiss(shape.shape_hash)}>
          Dismiss
        </button>
      </div>
    </div>
  );
}

function tokenKey(t: PiiToken): string {
  return `${t.kind}@${t.start}`;
}

/**
 * Apply the user's redaction choices to the raw example. Tokens are
 * processed right-to-left so earlier offsets aren't shifted by a
 * replacement of different length further along the string.
 */
function applyRedactions(
  raw: string,
  tokens: PiiToken[],
  redactions: Record<string, boolean>
): string {
  const sorted = [...tokens].sort((a, b) => b.start - a.start);
  let result = raw;
  for (const t of sorted) {
    if (redactions[tokenKey(t)]) {
      result = result.slice(0, t.start) + t.suggested_redaction + result.slice(t.end);
    }
  }
  return result;
}
