import React from 'react';

/**
 * BeamAlert — the outcome of an action.
 *
 * WHY THIS EXISTS. The system has semantic tokens (`--good`, `--warn`,
 * `--bad`) but nothing that carries a sentence in them. `BeamChip` is a
 * cell-sized status marker, and `Flatline` is an empty state, not a result.
 * Meanwhile every `/settings` server action redirects to `?status=…` or
 * `?error=…` and the reader needs to be told what happened.
 *
 * NO FILL AND NO BOX. A tinted banner is exactly the card-and-fill vocabulary
 * this language replaced, so the tone lives in a hairline rule and in the text
 * itself — the same way tone applies to a `Callout`'s figure and never to a
 * plate behind it.
 *
 * ERROR COPY STAYS LITERAL. The chrome is in-universe; anything that can go
 * wrong is not. "Authorisation service unavailable", never "Comm-link
 * disrupted" — the reader has to be able to act on it.
 */
export type BeamAlertTone = 'good' | 'warn' | 'bad';

export interface BeamAlertProps {
  tone?: BeamAlertTone;
  children: React.ReactNode;
  /**
   * `status` for a success, `alert` for a failure — an assertive live region
   * for the case where something the reader did did not work.
   */
  role?: 'status' | 'alert';
}

export function BeamAlert({
  tone = 'good',
  children,
  role,
}: BeamAlertProps) {
  const resolved = role ?? (tone === 'bad' ? 'alert' : 'status');
  return (
    <div
      className="hp-alert"
      data-tone={tone}
      role={resolved}
      aria-live={resolved === 'alert' ? 'assertive' : 'polite'}
    >
      {children}
    </div>
  );
}
