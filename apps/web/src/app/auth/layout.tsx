import React from 'react';
import { AppSectionSurface } from '@/components/projection/AppSectionSurface';
import {
  AuthTitle,
  AuthContext,
  AuthSteps,
} from './_components/AuthSteps';

/**
 * Projection frame for `/auth/**` — sign in, sign up, magic link, two-factor,
 * password reset, e-mail change and verification.
 *
 * FRAMED FROM THE LAYOUT, for the usual reason and one more. Every page in this
 * segment renders several `<main>` blocks — one per state (form / sent /
 * expired / done) — and wrapping each by hand across nine files is dozens of
 * edits on exactly the branches a test is least likely to visit. The layout
 * frames all of them at once.
 *
 * The extra reason: these are the LAST signed-out routes in the product. Every
 * other one is already a projection, so leaving auth behind would have meant a
 * visitor who clicked "Sign in" from a projection landed on the old flat shell
 * — the most jarring possible place for the seam to show.
 *
 * COVERAGE marks the kit's `Access.jsx` as INFERRED — a proposal, not a built
 * screen — so it is not followed wholesale. Two things in it are about the
 * product rather than the mock and are taken: per-step naming, and the idea
 * that the ways in belong together because they are one flow. Both live in
 * `_components/AuthSteps.tsx`, which records where they depart.
 *
 * NOT taken: the kit's six-digit OTP row (the product already has one, with
 * `autocomplete="one-time-code"` the mock lacks) and its links to every step
 * (most of them need a token). The state machines — TOTP, magic-link
 * redemption, reset tokens — are untouched.
 */
export default function AuthLayout({
  children,
}: {
  children: React.ReactNode;
}) {
  return (
    <AppSectionSurface
      crumb={[{ label: 'Site', href: '/' }, { label: 'Access' }]}
      // Per-step naming, from `Access.jsx`. The pane header said "Access" on
      // all nine routes before this — the same words whether you were signing
      // in, resetting a passphrase or confirming an address.
      title={<AuthTitle />}
      ctx={<AuthContext />}
    >
      <div className="hp-auth">
        <AuthSteps />
        {children}
      </div>
    </AppSectionSurface>
  );
}
