'use client';

/**
 * Tiny client island for the Withdraw button. The only reason this
 * needs client JS is the `window.confirm()` guard the spec calls for —
 * a misclick on Withdraw is destructive (the server immediately moves
 * the submission to `withdrawn`) so we ask before submitting.
 *
 * Everything else on the detail page (vote, flag) stays server-side
 * via plain `<form action={serverAction}>`.
 */

import { useState, useTransition } from 'react';

export function WithdrawForm({
  withdrawAction,
}: {
  withdrawAction: () => Promise<void>;
}) {
  const [pending, startTransition] = useTransition();
  const [confirmedAt, setConfirmedAt] = useState<number | null>(null);

  function handleClick() {
    // Two-step confirm pattern that doesn't depend on `window.confirm`
    // semantics differing across browsers — first click switches the
    // button into "Are you sure?" mode, second click within 5s fires.
    const now = Date.now();
    if (confirmedAt !== null && now - confirmedAt < 5_000) {
      setConfirmedAt(null);
      startTransition(() => {
        void withdrawAction();
      });
      return;
    }
    setConfirmedAt(now);
  }

  const armed = confirmedAt !== null;

  return (
    <button
      type="button"
      onClick={handleClick}
      disabled={pending}
      className="ss-btn ss-btn--ghost"
      style={{
        borderColor: armed ? 'var(--danger)' : undefined,
        color: armed ? 'var(--danger)' : undefined,
        width: '100%',
        justifyContent: 'flex-start',
      }}
    >
      {pending
        ? 'Withdrawing…'
        : armed
          ? 'Click again to confirm withdraw'
          : 'Withdraw submission'}
    </button>
  );
}
