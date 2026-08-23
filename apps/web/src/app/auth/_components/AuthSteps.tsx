'use client';

import React from 'react';
import Link from 'next/link';
import type { Route } from 'next';
import { usePathname } from 'next/navigation';

/**
 * The auth flow's own chrome — per-step naming, and the entry points.
 *
 * `Access.jsx` is one of the screens COVERAGE marks as INFERRED, so nothing
 * here is grounded in a built screen. It is still worth following on two
 * points, because both are about the product and not about the mock:
 *
 *   1. Every step gets its OWN title and subtitle — "Verify comm-link ·
 *      Confirm the address before syncing" — where the port framed all nine
 *      routes with a single shared "Access". The pane header therefore said
 *      the same words whether you were signing in, resetting a passphrase or
 *      confirming an address, which is the one place a header has real work to
 *      do: telling you which step of a multi-step flow you are standing on.
 *   2. "Every auth route reachable, because they are one flow." Sign-in and
 *      the three other ways in were separate dead ends.
 *
 * IN-UNIVERSE NOUNS FOR CHROME ONLY. "comm-link" is the product's word for an
 * email address and belongs in a title; it stays out of the pages' own copy,
 * where an error or an instruction has to be literal. That is the system's
 * rule — in-universe for chrome, literal for anything that can go wrong — not
 * a stylistic preference, and auth copy is the sharpest case for it.
 *
 * THE STRIP LISTS ENTRY POINTS, NOT STEPS. The kit links all seven because it
 * is one mock switching a `step` id, with no tokens and nothing to fail. In
 * the product, `reset-password`, `verify`, `email-change`, `totp-verify` and
 * the magic-link redeem are reached WITH A TOKEN — offering them as navigation
 * would send a reader to an error state and call it a destination, which is
 * the same fault as listing a redirect. What is left is the four things a
 * reader can honestly start from cold.
 */
const TITLES: Record<string, [string, string]> = {
  '/auth/login': ['Sign in', 'Your manifest is where you left it'],
  '/auth/signup': ['Create account', 'Beta · invite only'],
  '/auth/magic-link': ['Magic link', 'No password, one email'],
  '/auth/magic-link/redeem': ['Magic link', 'Opening your session'],
  '/auth/forgot-password': ['Reset passphrase', 'We send a single-use link'],
  '/auth/reset-password': ['Reset passphrase', 'Choose a new one'],
  '/auth/totp-verify': [
    'Authentication code',
    'Six digits from your authenticator',
  ],
  '/auth/verify': ['Verify comm-link', 'Confirm the address before syncing'],
  '/auth/email-change': ['Change comm-link', 'Both addresses are notified'],
  // `/auth/logout` is a route handler, not a page — it never renders chrome.
};

/** Only what a reader can legitimately begin without a token in hand. */
export const ENTRY_POINTS: readonly (readonly [string, string])[] = [
  ['/auth/login', 'Sign in'],
  ['/auth/signup', 'Create account'],
  ['/auth/magic-link', 'Magic link'],
  ['/auth/forgot-password', 'Reset'],
];

export function authStepFor(pathname: string): [string, string] {
  return TITLES[pathname] ?? ['Access', 'Sign in, or make an account'];
}

/** Every route in the segment, for the test that keeps the map complete. */
export const AUTH_STEP_ROUTES = Object.keys(TITLES);

function useAuthStep(): [string, string] {
  return authStepFor(usePathname() ?? '');
}

export function AuthTitle() {
  return <>{useAuthStep()[0]}</>;
}

export function AuthContext() {
  return <>{useAuthStep()[1]}</>;
}

export function AuthSteps() {
  const pathname = usePathname() ?? '';
  return (
    <nav className="hp-catstrip hp-authsteps" aria-label="Ways in">
      {ENTRY_POINTS.map(([href, label]) => (
        <Link
          key={href}
          href={href as Route}
          prefetch={false}
          className="hp-catchip"
          data-active={href === pathname ? 'true' : undefined}
          aria-current={href === pathname ? 'page' : undefined}
        >
          {label}
        </Link>
      ))}
    </nav>
  );
}
