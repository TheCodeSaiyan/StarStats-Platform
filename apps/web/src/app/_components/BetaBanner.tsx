import React from 'react';

/**
 * Slim invite-only notice for the auth pages, shown on the same
 * `gate_enabled` switch as the full-page `<BetaGate>` overlay.
 *
 * A banner rather than an interstitial: the overlay is right on the
 * landing page, where the visitor has no task in flight. On a sign-in
 * or sign-up form they DO, and putting a full-page interstitial in
 * front of it would block the thing they came to do.
 *
 * The copy differs per page because the gate does too. It is enforced
 * ONLY in the signup handler (`auth_routes.rs::signup`) — login is
 * untouched, so an existing account signs in exactly as before. Saying
 * anything else on /auth/login would be false.
 */
export type BetaBannerMode = 'login' | 'signup';

const COPY: Record<BetaBannerMode, { lead: string; detail: string }> = {
  login: {
    lead: 'StarStats is in invite-only beta.',
    // True: the gate guards signup only.
    detail: 'Existing accounts sign in as normal.',
  },
  signup: {
    lead: 'Invite-only beta.',
    detail:
      'You need an invite code to create an account. Join the waitlist and we will email you one.',
  },
};

export function BetaBanner({ mode }: { mode: BetaBannerMode }) {
  const { lead, detail } = COPY[mode];
  return (
    <div
      role="status"
      className="ss-card"
      style={{
        padding: '10px 14px',
        marginBottom: 16,
        borderColor: 'var(--accent)',
        display: 'flex',
        flexWrap: 'wrap',
        alignItems: 'baseline',
        gap: 8,
        fontSize: 13,
        lineHeight: 1.5,
      }}
    >
      <strong style={{ color: 'var(--accent)' }}>{lead}</strong>
      <span style={{ color: 'var(--fg-muted)' }}>{detail}</span>
    </div>
  );
}
