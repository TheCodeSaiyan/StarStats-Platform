import { useEffect, useState } from 'react';
import { api } from '../../api';
import { GhostButton, TrayCard } from './primitives';

/**
 * A way into chat until the tray has its own: opens the web's `/chat` in a
 * tray window (`open_chat_window`).
 *
 * Shown only when the server says chat is offered to this player
 * (`GET /v1/me/chat` → `offered`), which follows the same launch switch as
 * the web. While chat is staff-only, a player would otherwise be sent to a
 * page that is not found. Any failure hides the card rather than showing a
 * button that might not work.
 */
export function ChatLinkCard({ webOrigin }: { webOrigin: string | null }) {
  const [offered, setOffered] = useState(false);
  const [failed, setFailed] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;
    api
      .chatStatus()
      .then((s) => {
        if (!cancelled) setOffered(s.offered === true);
      })
      .catch(() => {
        if (!cancelled) setOffered(false);
      });
    return () => {
      cancelled = true;
    };
  }, []);

  if (!offered || !webOrigin) return null;
  return (
    <TrayCard
      title="Chat"
      right={
        <GhostButton
          type="button"
          onClick={() => {
            setFailed(null);
            api.openChatWindow().catch((e) => setFailed(String(e)));
          }}
        >
          Open chat
        </GhostButton>
      }
    >
      <p style={{ margin: 0, color: 'var(--fg-dim)' }}>
        Crew chat and messages with friends, end-to-end encrypted. Sign in once in the chat
        window; it remembers you.
      </p>
      {failed ? (
        <p role="alert" style={{ margin: '6px 0 0', color: 'var(--danger)' }}>
          Couldn&apos;t open chat: {failed}
        </p>
      ) : null}
    </TrayCard>
  );
}
