import { useEffect, useState } from 'react';
import { open as openShell } from '@tauri-apps/plugin-shell';
import { api } from '../../api';
import { GhostButton, TrayCard } from './primitives';

/**
 * A way into chat until the tray has its own: opens `/chat` on the web.
 *
 * Shown only when the server says chat is offered to this player
 * (`GET /v1/me/chat` → `offered`), which follows the same launch switch as
 * the web. While chat is staff-only, a player would otherwise be sent to a
 * page that is not found. Any failure hides the card rather than showing a
 * button that might not work.
 */
export function ChatLinkCard({ webOrigin }: { webOrigin: string | null }) {
  const [offered, setOffered] = useState(false);

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
            // openShell rejects if there is no default browser; nothing
            // useful to show beyond the button doing nothing.
            openShell(`${webOrigin}/chat`).catch(() => {});
          }}
        >
          Open chat
        </GhostButton>
      }
    >
      <p style={{ margin: 0, color: 'var(--fg-dim)' }}>
        Crew chat and messages with friends, end-to-end encrypted. Opens in your browser.
      </p>
    </TrayCard>
  );
}
