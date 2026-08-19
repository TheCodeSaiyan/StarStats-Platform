'use client';

/**
 * Full-page beta interstitial rendered over the (untouched) landing while
 * the invite-only gate is on. Layout decides whether to mount this at all
 * (gate_enabled && !dismissed, server-side); this component owns only the
 * in-session dismiss. Dismiss and a successful join both drop the
 * `ss_beta_dismissed` cookie so the layout stops rendering it on the next
 * navigation — one visit, remembered.
 *
 * Dismissible, so it is deliberately NOT a hard keyboard trap: Escape and
 * the "browse the site" button both close it. The underlying page stays in
 * the DOM behind it, so crawlers and readers still see the real content.
 */

import React, { useEffect, useRef, useState } from 'react';
import { WaitlistForm } from './WaitlistForm';

const DISMISS_COOKIE = 'ss_beta_dismissed';
// ~180 days.
const MAX_AGE = 60 * 60 * 24 * 180;

const SOURCE_KEY = 'ss_src';
const DEFAULT_SOURCE = 'beta-gate';

function setDismissed() {
  document.cookie = `${DISMISS_COOKIE}=1; path=/; max-age=${MAX_AGE}; samesite=lax`;
}

/**
 * Channel attribution for the waitlist. A `?src=reddit`-style param on
 * any URL wins and is remembered for the session, so someone who lands
 * from a launch post, dismisses the gate, browses, and joins later still
 * credits the post that sent them. Falls back to the fixed surface name.
 */
function channelSource(): string {
  try {
    const param = new URLSearchParams(window.location.search).get('src');
    if (param) {
      // Free-text channel label; cap it so a hostile URL can't stuff a
      // novel into the signup row.
      const cleaned = param.slice(0, 64);
      window.sessionStorage.setItem(SOURCE_KEY, cleaned);
      return cleaned;
    }
    return window.sessionStorage.getItem(SOURCE_KEY) ?? DEFAULT_SOURCE;
  } catch {
    // Storage can throw under hardened privacy settings; attribution is
    // never worth breaking the form over.
    return DEFAULT_SOURCE;
  }
}

export function BetaGate() {
  const [open, setOpen] = useState(true);
  // Resolved in an effect: this component SSRs once for the initial HTML,
  // and `window` isn't there. The value is only read at submit time, so
  // the late resolution can't cause a hydration mismatch.
  const [source, setSource] = useState(DEFAULT_SOURCE);
  const dialogRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    setSource(channelSource());
  }, []);

  const dismiss = () => {
    setDismissed();
    setOpen(false);
  };

  // Escape to dismiss; focus the dialog on mount.
  useEffect(() => {
    if (!open) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') dismiss();
    };
    document.addEventListener('keydown', onKey);
    dialogRef.current?.focus();
    return () => document.removeEventListener('keydown', onKey);
  }, [open]);

  if (!open) return null;

  return (
    <div
      className="ss-beta-gate"
      role="dialog"
      aria-modal="true"
      aria-labelledby="ss-beta-gate-title"
      ref={dialogRef}
      tabIndex={-1}
    >
      <div className="ss-beta-gate-panel">
        <div className="ss-placard">Private beta</div>
        <h1 id="ss-beta-gate-title">StarStats is in private beta.</h1>
        <p className="ss-lede">
          It&apos;s invite-only for now. Join the waitlist and we&apos;ll
          email you a signup link the moment a place opens up.
        </p>
        {/* onJoined lets the form tell us to remember the dismissal too. */}
        <WaitlistForm source={source} onJoined={setDismissed} />
        <button
          type="button"
          className="ss-beta-gate-dismiss"
          onClick={dismiss}
        >
          browse the site →
        </button>
      </div>
    </div>
  );
}
