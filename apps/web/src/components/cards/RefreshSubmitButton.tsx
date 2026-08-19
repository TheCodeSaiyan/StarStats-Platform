'use client';

/**
 * Tiny client wrapper around `useFormStatus` so dashboard refresh
 * forms (ProfileCard, OrgsCard) can show a "Refreshing…" pending
 * state without each card having to manage its own state. Audit
 * v2 §08 — the refresh CTA is the primary affordance on the card
 * once data is loaded, so it needs to feel snappy and accountable.
 *
 * MUST be rendered inside a <form action={...}> — `useFormStatus`
 * reads from the nearest form's transition state. Outside a form
 * the `pending` flag is always false and the button does nothing
 * extra; not harmful, just dead-weight.
 */

import { useFormStatus } from 'react-dom';

export function RefreshSubmitButton({
  idleLabel = 'Refresh now',
  pendingLabel = 'Refreshing…',
}: {
  idleLabel?: string;
  pendingLabel?: string;
}) {
  const { pending } = useFormStatus();
  return (
    <button
      type="submit"
      className="ss-btn ss-btn--ghost"
      disabled={pending}
      aria-busy={pending || undefined}
      style={{ fontSize: 12, padding: '6px 10px' }}
    >
      {pending ? pendingLabel : idleLabel}
    </button>
  );
}
