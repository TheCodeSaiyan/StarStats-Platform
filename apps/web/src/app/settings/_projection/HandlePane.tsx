import React from 'react';
import Link from 'next/link';
import type { Route } from 'next';
import { BeamButton, BeamInput, Plane } from 'holo';
import { ConfirmSubmitButton } from '@/components/forms/ConfirmSubmitButton';
import type { HandleStatusResponse } from '@/lib/api';

/**
 * Changing the RSI handle, and re-proving it, inside the "RSI handle
 * ownership" pane.
 *
 * Two steps, with no client state: the "Change handle" form is a plain GET
 * back to this page with `?rename=<handle>`, which renders the confirm step
 * listing what the change does; only "Confirm" runs the server action. So
 * the consequences are always on screen before anything changes, and a
 * reload of the confirm step is harmless.
 *
 * Every number shown (cooldown, hold) comes from the server's status
 * response, not from constants here.
 */

/** Same alphabet the server enforces (`validate_handle`). */
const HANDLE_RE = /^[A-Za-z0-9_-]{1,64}$/;

export function isValidHandle(h: string): boolean {
  return HANDLE_RE.test(h);
}

function formatDate(iso: string): string {
  const d = new Date(iso);
  return Number.isNaN(d.getTime()) ? iso : d.toLocaleDateString();
}

export function HandlePane({
  currentHandle,
  verified,
  status,
  candidate,
  changeAction,
}: {
  currentHandle: string;
  verified: boolean;
  /** `null` when the status call failed; the pane says so and offers nothing. */
  status: HandleStatusResponse | null;
  /** The handle the user asked to change to, from `?rename=`. */
  candidate: string | null;
  changeAction: (formData: FormData) => Promise<void>;
}) {
  if (!status) {
    return (
      <Plane tilt="flat" cap="Change handle" style={{ marginTop: 20 }}>
        <p className="hp-prose" style={{ marginTop: 0 }}>
          Couldn&apos;t load your handle settings. Refresh the page to try
          again.
        </p>
      </Plane>
    );
  }

  const reverify = verified ? (
    <p className="hp-prose">
      Want to prove the handle again?{' '}
      <Link href={'/settings?pane=rsi&reverify=1#rsi' as Route}>
        Re-verify {currentHandle}
      </Link>
      . You stay verified while you do it, and a failed check doesn&apos;t
      change that.
    </p>
  ) : null;

  if (candidate !== null && isValidHandle(candidate) && candidate !== currentHandle) {
    return (
      <Plane tilt="flat" cap="Confirm handle change" style={{ marginTop: 20 }}>
        <p className="hp-prose" style={{ marginTop: 0 }}>
          Change your handle from <span className="val">{currentHandle}</span>{' '}
          to <span className="val">{candidate}</span>? This is what happens:
        </p>
        <ul className="hp-steps">
          <li>
            {verified
              ? `${currentHandle} is verified now. ${candidate} won't be until you prove it with the RSI bio code, which you'll do straight after this.`
              : `You'll prove ${candidate} with the RSI bio code straight after this.`}
          </li>
          <li>
            {currentHandle} stays reserved for you for{' '}
            {status.reservation_days} days. Profile and share links that use it
            send people to {candidate} until then.
          </li>
          <li>
            {status.free_change_available
              ? `This is your one free change, because the account has never been verified. After it, you can't change your handle again for ${status.cooldown_days} days.`
              : `You can't change your handle again for ${status.cooldown_days} days.`}
          </li>
          <li>
            Your stats, shares, friends and chat stay with you. Paired devices
            switch to {candidate} on their next sync; you don&apos;t need to
            pair them again.
          </li>
        </ul>
        <div style={{ display: 'flex', flexWrap: 'wrap', gap: 10, marginTop: 16 }}>
          <form action={changeAction}>
            <input type="hidden" name="new_handle" value={candidate} />
            <ConfirmSubmitButton
              className="hp-btn hp-btn--primary"
              pendingLabel="Changing…"
            >
              Change handle to {candidate}
            </ConfirmSubmitButton>
          </form>
          <BeamButton href="/settings?pane=rsi#rsi" variant="ghost">
            Keep {currentHandle}
          </BeamButton>
        </div>
      </Plane>
    );
  }

  if (status.cooldown_until) {
    return (
      <Plane tilt="flat" cap="Change handle" style={{ marginTop: 20 }}>
        <p className="hp-prose" style={{ marginTop: 0 }}>
          You changed your handle recently. You can change it again on{' '}
          <span className="val">{formatDate(status.cooldown_until)}</span>.
          {status.previous_handle && status.previous_handle_reserved_until
            ? ` ${status.previous_handle} is reserved for you until ${formatDate(status.previous_handle_reserved_until)}.`
            : ''}
        </p>
        {reverify}
      </Plane>
    );
  }

  return (
    <Plane tilt="flat" cap="Change handle" style={{ marginTop: 20 }}>
      <p className="hp-prose" style={{ marginTop: 0 }}>
        Typed it wrong, or changed your name on RSI? Enter the handle exactly as
        it appears on your RSI profile. You&apos;ll see what changes before
        anything happens.
      </p>
      {/* GET, not a server action: it only opens the confirm step. */}
      <form method="get" action="/settings" className="hp-formrow">
        <input type="hidden" name="pane" value="rsi" />
        <BeamInput
          id="rename-handle"
          label="New RSI handle"
          name="rename"
          required
          pattern="[A-Za-z0-9_\-]{1,64}"
          title="Letters, digits, _ and -, up to 64 characters"
          autoComplete="off"
          spellCheck={false}
        />
        <BeamButton type="submit" variant="ghost">
          Change handle…
        </BeamButton>
      </form>
      {reverify}
    </Plane>
  );
}
