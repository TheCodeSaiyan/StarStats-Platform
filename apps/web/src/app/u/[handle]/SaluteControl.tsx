import React from 'react';
import { ConfirmSubmitButton } from '@/components/forms/ConfirmSubmitButton';
import { ApiCallError, getMySalutes, getSaluteSummary, type SaluteSummary } from '@/lib/api';
import { logger } from '@/lib/logger';
import { saluteAction } from './salute-actions';

const ERRORS: Record<string, string> = {
  rsi_handle_not_verified: 'Verify your RSI handle to salute. Calibrate → RSI handle.',
  rate_limited: 'That is a lot of salutes. Try again in a minute.',
  not_found: 'This profile is not one you can salute.',
  spicedb_unavailable: 'Salutes are unavailable right now. Try again shortly.',
  unexpected: 'Something went wrong. Try again.',
};

/**
 * o7 salutes on a profile: the public count, a Salute toggle for a signed-in
 * visitor, and for the owner, which of their friends saluted (never anyone
 * else's name). Renders nothing when the count cannot be read, so a salute
 * outage costs the chip and not the page.
 */
export async function SaluteControl({
  handle,
  token,
  isOwner,
  error,
}: {
  handle: string;
  token: string | null;
  isOwner: boolean;
  error?: string;
}) {
  let summary: SaluteSummary;
  try {
    summary = await getSaluteSummary(handle, token ?? undefined);
  } catch (e) {
    if (!(e instanceof ApiCallError) || e.status !== 404) {
      logger.warn({ err: e, call: 'profile.salutes' }, 'salute count fetch failed');
    }
    return null;
  }

  let friends: string[] = [];
  if (isOwner && token) {
    try {
      friends = (await getMySalutes(token)).friends;
    } catch (e) {
      logger.warn({ err: e, call: 'profile.my_salutes' }, 'friend salutes fetch failed');
    }
  }

  const saluted = summary.saluted_by_me === true;
  const count = summary.count;
  return (
    <>
      <span
        className="hp-chip"
        data-testid="salute-count"
        title={`${count} ${count === 1 ? 'salute' : 'salutes'}`}
      >
        o7 · {count}
      </span>
      {!isOwner && token ? (
        <form action={saluteAction} style={{ margin: 0 }}>
          <input type="hidden" name="handle" value={handle} />
          <input type="hidden" name="intent" value={saluted ? 'unsalute' : 'salute'} />
          <ConfirmSubmitButton
            className="hp-btn hp-btn--ghost"
            aria-pressed={saluted}
            pendingLabel="o7…"
          >
            {saluted ? 'Saluted' : 'Salute'}
          </ConfirmSubmitButton>
        </form>
      ) : null}
      {isOwner && friends.length > 0 ? (
        <span className="hp-chip" data-testid="salute-friends">
          Saluted by {friends.map((f) => `@${f}`).join(', ')}
        </span>
      ) : null}
      {error ? (
        <span className="hp-chip bad" role="alert" data-testid="salute-error">
          {ERRORS[error] ?? ERRORS.unexpected}
        </span>
      ) : null}
    </>
  );
}
