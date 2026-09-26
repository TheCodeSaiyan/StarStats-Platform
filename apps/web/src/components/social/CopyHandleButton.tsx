'use client';

import React from 'react';
import { BeamButton } from 'holo';

/**
 * RSI's own friend management. It is a settings page, not a per-player one,
 * so there is no way to deep-link to "add this person" — which is why the
 * handle is copied first and the reader pastes it into RSI's add-friend box.
 */
export const RSI_FRIENDS_URL =
  'https://robertsspaceindustries.com/spectrum/settings/friends';

type Status = 'idle' | 'copied' | 'failed';

/**
 * Copy a player's handle so it can be pasted into the game or into RSI.
 *
 * Only a VERIFIED handle is copyable. An unverified StarStats handle is just a
 * name someone typed at sign-up; copying it into an in-game invite would reach
 * whoever really owns that name on RSI. So the button is disabled with the
 * reason in its tooltip, rather than hidden, so the reader knows why.
 *
 * `addInGame` also opens RSI's friends page. The clipboard write is STARTED
 * before the new tab opens: the browser refuses clipboard writes from a
 * document that has lost focus, and opening a tab takes the focus away.
 */
export function CopyHandleButton({
  handle,
  verified,
  addInGame = false,
}: {
  handle: string;
  verified: boolean;
  addInGame?: boolean;
}) {
  const [status, setStatus] = React.useState<Status>('idle');

  React.useEffect(() => {
    if (status === 'idle') return;
    const t = window.setTimeout(() => setStatus('idle'), 2500);
    return () => window.clearTimeout(t);
  }, [status]);

  const onClick = async () => {
    let copy: Promise<void>;
    try {
      copy = navigator.clipboard.writeText(handle);
    } catch {
      copy = Promise.reject(new Error('clipboard unavailable'));
    }
    if (addInGame) {
      window.open(RSI_FRIENDS_URL, '_blank', 'noopener,noreferrer');
    }
    try {
      await copy;
      setStatus('copied');
    } catch {
      setStatus('failed');
    }
  };

  const label = addInGame ? 'Add in game' : 'Copy handle';
  const title = !verified
    ? 'Handle not verified with RSI'
    : addInGame
      ? `Copy ${handle} and open RSI friend management`
      : `Copy ${handle} to paste in game`;

  return (
    <span className="ss-copy-handle">
      <BeamButton
        type="button"
        variant="ghost"
        disabled={!verified}
        aria-disabled={!verified}
        title={title}
        onClick={verified ? onClick : undefined}
      >
        {label}
      </BeamButton>
      {/* Announced, not just shown: a sighted reader sees the label change, a
          screen-reader user would otherwise get no confirmation at all. */}
      <span role="status" aria-live="polite" className="ss-copy-handle__status">
        {status === 'copied'
          ? addInGame
            ? "Handle copied — paste it into RSI's add-friend box"
            : 'Handle copied'
          : status === 'failed'
            ? 'Copy failed — select the handle and copy it'
            : ''}
      </span>
    </span>
  );
}
