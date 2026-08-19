'use client';

import React, { useId, useState } from 'react';
import { joinWaitlistAction } from '@/app/_actions/waitlist';
import type { JoinWaitlistResult } from '@/app/_actions/waitlist';

const CONTACT_EMAIL = 'dojo@thecodesaiyan.io';

/**
 * Waitlist capture for the landing page.
 *
 * `source` is free-text channel attribution ("reddit", "spectrum") so a
 * launch can tell which post actually sent people. Omit it and the signup
 * is still a signup.
 *
 * On success the form is replaced by its result rather than left sitting
 * there: someone who has just been told they are number 12 in the queue
 * should not be looking at a button that invites them to join again.
 */
export function WaitlistForm({
  source,
  onJoined,
}: {
  source?: string;
  onJoined?: () => void;
}) {
  const emailId = useId();
  const [result, setResult] = useState<JoinWaitlistResult | null>(null);
  const [pending, setPending] = useState(false);

  async function onSubmit(e: React.FormEvent<HTMLFormElement>) {
    e.preventDefault();
    if (pending) return;
    setPending(true);
    try {
      const fd = new FormData(e.currentTarget);
      const res = await joinWaitlistAction(fd);
      setResult(res);
      if (res.ok) onJoined?.();
    } finally {
      setPending(false);
    }
  }

  if (result?.ok) {
    return (
      <div
        className="ss-card"
        style={{ padding: 'var(--s5) var(--s6)', marginTop: 'var(--s4)' }}
      >
        <div className="ss-placard" style={{ marginBottom: 'var(--s2)' }}>
          {result.position === null ? "You're in" : "You're on the list"}
        </div>
        <p style={{ margin: 0, fontSize: 'var(--fs-base)', lineHeight: 1.65 }}>
          {result.position === null ? (
            <>Check your email — your signup link is on its way.</>
          ) : (
            <>
              You&rsquo;re number{' '}
              <strong style={{ fontSize: 'var(--fs-lg)' }}>
                {result.position}
              </strong>{' '}
              in the queue. We&rsquo;ll email you the moment a place opens
              up.
            </>
          )}
        </p>
      </div>
    );
  }

  const isBadEmail = result && !result.ok && result.error === 'invalid_email';

  return (
    <form
      onSubmit={onSubmit}
      style={{ marginTop: 'var(--s4)' }}
      noValidate
    >
      {source ? <input type="hidden" name="source" value={source} /> : null}
      <label
        htmlFor={emailId}
        style={{
          display: 'block',
          marginBottom: 'var(--s2)',
          fontSize: 'var(--fs-sm)',
          color: 'var(--fg-muted)',
        }}
      >
        Email
      </label>
      <div style={{ display: 'flex', gap: 'var(--s2)', flexWrap: 'wrap' }}>
        <input
          id={emailId}
          name="email"
          aria-label="Waitlist email"
          type="email"
          required
          autoComplete="email"
          placeholder="you@example.com"
          disabled={pending}
          style={{ flex: '1 1 16rem', minWidth: 0 }}
        />
        <button type="submit" className="ss-btn" disabled={pending}>
          {pending ? 'Joining…' : 'Join the waitlist'}
        </button>
      </div>

      {result && !result.ok ? (
        <p
          role="alert"
          style={{
            marginTop: 'var(--s3)',
            marginBottom: 0,
            color: 'var(--fg-muted)',
            fontSize: 'var(--fs-sm)',
          }}
        >
          {isBadEmail ? (
            <>That doesn&rsquo;t look like an email address — check it and
            try again.</>
          ) : (
            <>
              Something went wrong on our end. Try again, or email{' '}
              <a href={`mailto:${CONTACT_EMAIL}`}>{CONTACT_EMAIL}</a> and
              I&rsquo;ll add you by hand.
            </>
          )}
        </p>
      ) : null}
    </form>
  );
}
