'use client';

/**
 * The web chat client (social phase 6): crew rooms and friend DMs,
 * end-to-end encrypted, text only.
 *
 * matrix-js-sdk with its Rust crypto (WebAssembly) runs here in the
 * browser; StarStats never sees a message. The client signs in with a
 * one-minute token from the API, then keeps its device (and keys) between
 * visits: the session in localStorage, the encryption store in IndexedDB,
 * encrypted with a key wrapped by a non-extractable WebCrypto key
 * (`lib/chat/storage-key`). Sign-out forgets all of it.
 *
 * Two rules learned against a real Synapse:
 * - a room can be sent to only once sync reports it JOINED; before that the
 *   crypto layer does not know it is encrypted and refuses to send;
 * - messages are shown as plain text (`lib/chat/render`), never their HTML.
 */
import React from 'react';
import type { MatrixClient, MatrixEvent, Room } from 'matrix-js-sdk';
import type { MyChatRoom } from '@/lib/api';
import { chatLoginAction, reportChatAction } from '@/app/chat/actions';
import { MessageText } from './MessageText';
import { localpartOf, MAX_MESSAGE_CHARS, toLine, type ChatLine } from '@/lib/chat/render';
import { CHAT_SESSION_KEY, cryptoStorePrefix, sessionFor, type StoredChatSession } from '@/lib/chat/session';
import { chatStoreKey } from '@/lib/chat/storage-key';

type Phase =
  | { kind: 'connecting' }
  | { kind: 'ready' }
  | { kind: 'error'; message: string };

const ERRORS: Record<string, string> = {
  rsi_handle_not_verified: 'Verify your RSI handle to use chat.',
  age_not_declared: 'Confirm your age to use chat.',
  chat_restricted: 'Your account is restricted from chat.',
  chat_unavailable: 'Chat is not available right now.',
  rate_limited: 'Too many sign-ins. Wait a minute and refresh.',
};

async function signIn(handle: string): Promise<StoredChatSession> {
  let stored: StoredChatSession | null = null;
  try {
    stored = sessionFor(localStorage.getItem(CHAT_SESSION_KEY), handle);
  } catch {
    // Storage blocked: sign in fresh each visit.
  }
  if (stored) return stored;
  const res = await chatLoginAction();
  if (!res.ok) throw new Error(ERRORS[res.error] ?? 'Could not sign in to chat.');
  const sdk = await import('matrix-js-sdk');
  const bare = sdk.createClient({ baseUrl: res.login.homeserver_url });
  const login = await bare.loginRequest({
    type: 'org.matrix.login.jwt',
    token: res.login.token,
    initial_device_display_name: 'StarStats web',
  });
  const fresh: StoredChatSession = {
    handle: handle.toLowerCase(),
    userId: login.user_id,
    deviceId: login.device_id,
    accessToken: login.access_token,
    homeserverUrl: res.login.homeserver_url,
  };
  try {
    localStorage.setItem(CHAT_SESSION_KEY, JSON.stringify(fresh));
  } catch {
    // Not kept: this visit only.
  }
  return fresh;
}

function roomLabel(r: MyChatRoom): string {
  return r.kind === 'dm' ? `@${r.other_handle ?? 'player'}` : 'Crew chat';
}

export function ChatApp({
  handle,
  rooms,
  initialRoom,
}: {
  handle: string;
  rooms: MyChatRoom[];
  initialRoom?: string;
}) {
  const [phase, setPhase] = React.useState<Phase>({ kind: 'connecting' });
  const [selected, setSelected] = React.useState<string | null>(
    initialRoom && rooms.some((r) => r.room_id === initialRoom)
      ? initialRoom
      : (rooms[0]?.room_id ?? null),
  );
  const [lines, setLines] = React.useState<ChatLine[]>([]);
  const [canSend, setCanSend] = React.useState(false);
  const [draft, setDraft] = React.useState('');
  const [sending, setSending] = React.useState(false);
  // Reporting: whose messages, which ones ticked, and the outcome.
  const [reportFor, setReportFor] = React.useState<string | null>(null);
  const [ticked, setTicked] = React.useState<Set<string>>(new Set());
  const [reason, setReason] = React.useState('harassment');
  const [details, setDetails] = React.useState('');
  const [reportNote, setReportNote] = React.useState<string | null>(null);
  const clientRef = React.useRef<MatrixClient | null>(null);
  const [myId, setMyId] = React.useState('');

  const refresh = React.useCallback(() => {
    const client = clientRef.current;
    if (!client || !selected) return;
    const room: Room | null = client.getRoom(selected);
    const events: MatrixEvent[] = room ? room.getLiveTimeline().getEvents() : [];
    setLines(events.map((e) => toLine(e)).filter((l) => l.kind !== 'hidden'));
  }, [selected]);

  // Start the client once.
  React.useEffect(() => {
    let cancelled = false;
    let client: MatrixClient | null = null;
    (async () => {
      try {
        const session = await signIn(handle);
        const sdk = await import('matrix-js-sdk');
        client = sdk.createClient({
          baseUrl: session.homeserverUrl,
          accessToken: session.accessToken,
          userId: session.userId,
          deviceId: session.deviceId,
        });
        setMyId(session.userId);
        client.on(sdk.HttpApiEvent.SessionLoggedOut, () => {
          // The token was revoked: forget it so the next load signs in again.
          try {
            localStorage.removeItem(CHAT_SESSION_KEY);
          } catch {
            // Nothing kept.
          }
          setPhase({ kind: 'error', message: 'Your chat session ended. Refresh to sign in again.' });
        });
        await client.initRustCrypto({
          cryptoDatabasePrefix: cryptoStorePrefix(session.userId),
          storageKey: await chatStoreKey(session.userId),
        });
        await client.startClient({ initialSyncLimit: 30, lazyLoadMembers: true });
        await new Promise<void>((resolve) => {
          const onSync = (state: string) => {
            if (state === 'PREPARED' || state === 'SYNCING') {
              client!.off(sdk.ClientEvent.Sync, onSync);
              resolve();
            }
          };
          client!.on(sdk.ClientEvent.Sync, onSync);
        });
        if (cancelled) {
          client.stopClient();
          return;
        }
        clientRef.current = client;
        setPhase({ kind: 'ready' });
      } catch (e) {
        if (!cancelled) {
          setPhase({
            kind: 'error',
            message: e instanceof Error ? e.message : 'Could not start chat.',
          });
        }
      }
    })();
    return () => {
      cancelled = true;
      client?.stopClient();
      clientRef.current = null;
    };
  }, [handle]);

  // Follow the selected room: join an invite, wait until it is joined and
  // encrypted before allowing a send, and redraw on new or decrypted events.
  React.useEffect(() => {
    const client = clientRef.current;
    if (phase.kind !== 'ready' || !client || !selected) return;
    let stop = false;
    setCanSend(false);
    (async () => {
      const room = client.getRoom(selected);
      if (!room || room.getMyMembership() === 'invite') {
        try {
          await client.joinRoom(selected);
        } catch {
          setPhase({ kind: 'error', message: 'Could not open that chat.' });
          return;
        }
      }
      for (let i = 0; i < 80 && !stop; i++) {
        const r = client.getRoom(selected);
        const joined = r?.getMyMembership() === 'join';
        if (joined && (await client.getCrypto()?.isEncryptionEnabledInRoom(selected))) {
          if (!stop) setCanSend(true);
          break;
        }
        await new Promise((res) => setTimeout(res, 250));
      }
      if (!stop) refresh();
    })();
    const sdkEvents = import('matrix-js-sdk');
    let unbind = () => {};
    void sdkEvents.then((sdk) => {
      const onTimeline = (_e: MatrixEvent, room?: Room) => {
        if (room?.roomId === selected) refresh();
      };
      const onDecrypted = () => refresh();
      client.on(sdk.RoomEvent.Timeline, onTimeline);
      client.on(sdk.MatrixEventEvent.Decrypted, onDecrypted);
      unbind = () => {
        client.off(sdk.RoomEvent.Timeline, onTimeline);
        client.off(sdk.MatrixEventEvent.Decrypted, onDecrypted);
      };
    });
    return () => {
      stop = true;
      unbind();
    };
  }, [phase.kind, selected, refresh]);

  const send = async (ev: React.FormEvent) => {
    ev.preventDefault();
    const client = clientRef.current;
    const text = draft.trim().slice(0, MAX_MESSAGE_CHARS);
    if (!client || !selected || !text || !canSend) return;
    setSending(true);
    try {
      await client.sendTextMessage(selected, text);
      setDraft('');
    } catch {
      setPhase({ kind: 'error', message: 'That message was not sent. Refresh and try again.' });
    } finally {
      setSending(false);
    }
  };

  const startReport = (sender: string, eventId: string) => {
    setReportFor(sender);
    setTicked(new Set([eventId]));
    setReportNote(null);
  };

  const sendReport = async (ev: React.FormEvent) => {
    ev.preventDefault();
    if (!reportFor || !selected) return;
    const messages = lines
      .filter((l): l is Extract<ChatLine, { kind: 'text' }> => l.kind === 'text')
      .filter((l) => l.sender === reportFor && ticked.has(l.id))
      .map((l) => ({
        event_id: l.id,
        sender: l.sender,
        sent_at: new Date(l.ts).toISOString(),
        text: l.text,
      }));
    if (messages.length === 0) return;
    const res = await reportChatAction({
      room_id: selected,
      reported_handle: localpartOf(reportFor),
      reason: reason as 'harassment' | 'spam' | 'scam' | 'illegal_content' | 'other',
      details: details.trim() || undefined,
      messages,
    });
    if (res.ok) {
      setReportFor(null);
      setTicked(new Set());
      setDetails('');
      setReportNote('Reported. Moderators see only the messages you ticked.');
    } else {
      setReportNote(
        res.error === 'rate_limited'
          ? 'You have sent a lot of reports today. Try again tomorrow.'
          : 'That report was not sent. Try again.',
      );
    }
  };

  if (rooms.length === 0) {
    return (
      <p className="hp-prose">
        No chats yet. Message a friend with New message, or join a crew on Looking for
        Group: its crew get a chat room when the host accepts them.
      </p>
    );
  }

  return (
    <div className="ss-chat" data-testid="chat-app">
      <nav className="ss-chat__rooms" aria-label="Your chats">
        {rooms.map((r) => (
          <button
            key={r.room_id}
            type="button"
            className={r.room_id === selected ? 'hp-btn' : 'hp-btn hp-btn--ghost'}
            aria-current={r.room_id === selected ? 'true' : undefined}
            onClick={() => setSelected(r.room_id)}
          >
            {roomLabel(r)}
          </button>
        ))}
      </nav>
      <section className="ss-chat__room" aria-live="polite">
        {phase.kind === 'connecting' ? (
          <p className="hp-prose">Connecting securely…</p>
        ) : phase.kind === 'error' ? (
          <p className="hp-prose" role="alert">
            {phase.message}
          </p>
        ) : (
          <>
            <ol className="ss-chat__lines" data-testid="chat-lines">
              {lines.map((l) =>
                l.kind === 'text' ? (
                  <li key={l.id} className="ss-chat__line">
                    {reportFor && l.sender === reportFor ? (
                      <input
                        type="checkbox"
                        aria-label="Include this message in the report"
                        checked={ticked.has(l.id)}
                        onChange={(e) => {
                          const next = new Set(ticked);
                          if (e.target.checked) next.add(l.id);
                          else next.delete(l.id);
                          setTicked(next);
                        }}
                      />
                    ) : null}
                    <span>
                      <strong>{l.sender === myId ? 'You' : `@${localpartOf(l.sender)}`}</strong>{' '}
                      <MessageText text={l.text} />
                    </span>
                    {l.sender !== myId && !reportFor ? (
                      <button
                        type="button"
                        className="ss-chat__report"
                        onClick={() => startReport(l.sender, l.id)}
                      >
                        Report
                      </button>
                    ) : null}
                  </li>
                ) : l.kind === 'undecryptable' ? (
                  <li key={l.id} className="ss-chat__muted">
                    <strong>@{localpartOf(l.sender)}</strong> sent a message this browser cannot
                    decrypt (it was sent before this browser joined, or its keys are elsewhere).
                  </li>
                ) : null,
              )}
            </ol>
            {reportNote ? (
              <p className="ss-chat__muted" role="status">
                {reportNote}
              </p>
            ) : null}
            {reportFor ? (
              <form
                onSubmit={(e) => void sendReport(e)}
                className="hp-formcol ss-chat__reportform"
                data-testid="chat-report-form"
              >
                <p className="hp-prose" style={{ margin: 0 }}>
                  Report @{localpartOf(reportFor)}. Tick the messages to include: only those are
                  sent, in plain text, to StarStats moderators. Nothing else in this chat is.
                </p>
                <label className="hp-formcol">
                  Reason
                  <select className="hp-input" value={reason} onChange={(e) => setReason(e.target.value)}>
                    <option value="harassment">Harassment or abuse</option>
                    <option value="scam">Scam or phishing</option>
                    <option value="spam">Spam</option>
                    <option value="illegal_content">Illegal content</option>
                    <option value="other">Something else</option>
                  </select>
                </label>
                <label className="hp-formcol">
                  Anything else moderators should know (optional)
                  <textarea
                    className="hp-input"
                    rows={2}
                    maxLength={500}
                    value={details}
                    onChange={(e) => setDetails(e.target.value)}
                  />
                </label>
                <span className="hp-formrow">
                  <button type="submit" className="hp-btn" disabled={ticked.size === 0}>
                    Send report ({ticked.size})
                  </button>
                  <button type="button" className="hp-btn hp-btn--ghost" onClick={() => setReportFor(null)}>
                    Cancel
                  </button>
                </span>
              </form>
            ) : null}
            <form onSubmit={(e) => void send(e)} className="hp-formrow">
              <label htmlFor="chat-draft" className="ss-sr-only">
                Message
              </label>
              <input
                id="chat-draft"
                className="hp-input"
                value={draft}
                maxLength={MAX_MESSAGE_CHARS}
                disabled={!canSend || sending}
                placeholder={canSend ? 'Message' : 'Opening encrypted chat…'}
                onChange={(e) => setDraft(e.target.value)}
                autoComplete="off"
              />
              <button type="submit" className="hp-btn" disabled={!canSend || sending || !draft.trim()}>
                Send
              </button>
            </form>
          </>
        )}
        <p className="ss-chat__muted">
          End-to-end encrypted: StarStats cannot read these messages. History is kept in this
          browser; signing out or clearing site data removes it here.
        </p>
      </section>
    </div>
  );
}
