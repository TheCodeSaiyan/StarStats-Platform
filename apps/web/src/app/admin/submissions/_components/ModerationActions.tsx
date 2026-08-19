'use client';

/**
 * Moderation action buttons rendered alongside each submission row in
 * the admin queue.
 *
 * Bearer-token leak: the user JWT is server-only (HttpOnly cookie via
 * lib/session.ts). We never pass it into a client component. Instead,
 * the parent page defines `'use server'` actions that read the cookie
 * server-side, call the API, and `revalidatePath('/admin/submissions')`
 * to re-fetch the row. Those actions are passed in as props and bound
 * to native `<form action={...}>` — no fetch wiring, no token in JS.
 *
 * Action visibility by status:
 *   review  → Accept / Reject
 *   flagged → Accept / Reject / Dismiss flag
 *   other   → muted "(no actions available)" string
 *
 * The Reject path uses `<details>` to keep a textarea hidden until the
 * moderator opens it, so the per-row chrome stays compact.
 */

import { useId } from 'react';

type ActionFn = (formData: FormData) => void | Promise<void>;

interface Props {
  submissionId: string;
  status: string;
  acceptAction: ActionFn;
  rejectAction: ActionFn;
  dismissAction: ActionFn;
}

export function ModerationActions({
  submissionId,
  status,
  acceptAction,
  rejectAction,
  dismissAction,
}: Props) {
  const rejectFormId = useId();

  const showAccept = status === 'review' || status === 'flagged';
  const showReject = status === 'review' || status === 'flagged';
  const showDismiss = status === 'flagged';

  if (!showAccept && !showReject && !showDismiss) {
    return (
      <span style={{ color: 'var(--fg-dim)', fontSize: 12 }}>
        (no actions available)
      </span>
    );
  }

  return (
    <div
      style={{
        display: 'flex',
        flexWrap: 'wrap',
        gap: 8,
        alignItems: 'flex-start',
      }}
    >
      {showAccept && (
        <form action={acceptAction}>
          <input type="hidden" name="id" value={submissionId} />
          <button
            type="submit"
            className="ss-btn ss-btn--primary"
            style={{ fontSize: 12, padding: '6px 12px' }}
          >
            Accept
          </button>
        </form>
      )}

      {showReject && (
        <details style={{ position: 'relative' }}>
          <summary
            className="ss-btn ss-btn--ghost"
            style={{
              fontSize: 12,
              padding: '6px 12px',
              listStyle: 'none',
              cursor: 'pointer',
            }}
          >
            Reject
          </summary>
          <form
            id={rejectFormId}
            action={rejectAction}
            style={{
              marginTop: 6,
              padding: 10,
              background: 'var(--bg-elev)',
              border: '1px solid var(--border)',
              borderRadius: 'var(--r-md)',
              display: 'flex',
              flexDirection: 'column',
              gap: 8,
              minWidth: 240,
            }}
          >
            <input type="hidden" name="id" value={submissionId} />
            <label
              htmlFor={`${rejectFormId}-reason`}
              className="ss-eyebrow"
              style={{ fontSize: 10 }}
            >
              Reason (visible to submitter)
            </label>
            <textarea
              id={`${rejectFormId}-reason`}
              name="reason"
              required
              minLength={3}
              maxLength={500}
              rows={3}
              placeholder="Why is this rule being rejected?"
              style={{
                width: '100%',
                background: 'var(--bg)',
                color: 'var(--fg)',
                border: '1px solid var(--border)',
                borderRadius: 'var(--r-sm)',
                padding: 8,
                fontSize: 12,
                fontFamily: 'inherit',
                resize: 'vertical',
              }}
            />
            <button
              type="submit"
              className="ss-btn ss-btn--danger"
              style={{
                fontSize: 12,
                padding: '6px 12px',
                alignSelf: 'flex-start',
              }}
            >
              Confirm reject
            </button>
          </form>
        </details>
      )}

      {showDismiss && (
        <form action={dismissAction}>
          <input type="hidden" name="id" value={submissionId} />
          <button
            type="submit"
            className="ss-btn ss-btn--ghost"
            style={{ fontSize: 12, padding: '6px 12px' }}
          >
            Dismiss flag
          </button>
        </form>
      )}
    </div>
  );
}
