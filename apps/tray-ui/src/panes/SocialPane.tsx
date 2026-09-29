/**
 * Tray UI — Social pane: friends, requests, notifications, blocks, mutes
 * and the toast settings.
 *
 * Every call goes Rust-side (`social_*` commands) because the WebView CSP
 * blocks cross-origin fetch. The tray authenticates with its device token;
 * the server resolves that to the owning player.
 *
 * "Add in game" copies the friend's handle and opens RSI's friend settings
 * in the browser. Only an RSI-verified handle is copyable: an unverified
 * StarStats handle is just a name typed at sign-up, and pasting it into an
 * in-game invite would reach whoever really owns it.
 *
 * No `position: fixed` descendants, so the `ss-screen-enter` transform trap
 * (portal fixed elements to `document.body`) does not apply here.
 */

import { ChatLinkCard } from '../components/tray/ChatLinkCard';
import { useCallback, useEffect, useState, type FormEvent } from 'react';
import { listen } from '@tauri-apps/api/event';
import { open as openShell } from '@tauri-apps/plugin-shell';
import {
  api,
  type FriendPresence,
  type FriendRequestPolicy,
  type FriendsResponse,
  type ListedHandle,
  type NotificationsResponse,
  type SocialNotification,
  type PresenceLevel,
  type SocialPrefs,
} from '../api';
import { commendLabel } from '../lib/commends';
import { presenceLabel } from '../lib/presence';
import {
  Banner,
  DangerButton,
  GhostButton,
  PrimaryButton,
  TextInput,
  TrayCard,
} from '../components/tray/primitives';
import { relativeTimeSince } from './WhatsNewPane';

export const RSI_FRIENDS_URL =
  'https://robertsspaceindustries.com/spectrum/settings/friends';

/** Server error codes → copy. Unknown codes fall back to the raw message. */
const ERROR_COPY: Record<string, string> = {
  invalid_handle: 'That handle looks invalid — letters, digits, underscores and dashes only.',
  user_not_found: 'No StarStats account exists for that handle.',
  cannot_friend_self: "You can't send yourself a friend request.",
  already_friends: 'You are already friends.',
  request_pending: 'You already have a request waiting with them.',
  you_blocked_user: 'You have blocked that user. Unblock them first.',
  not_accepting_requests: 'That user is not accepting friend requests.',
  rate_limited: "You've sent a lot of requests today. Try again tomorrow.",
  request_not_found: 'That request no longer exists.',
  request_not_pending: 'That request has already been answered.',
  not_friends: 'You are not friends with that user.',
};

export function describeError(e: unknown): string {
  const msg = String(e);
  if (msg.includes('not paired')) return 'Pair this tray to use friends.';
  return ERROR_COPY[msg] ?? msg;
}

/**
 * Copy the handle, and for "add in game" open RSI afterwards. The copy is
 * started first: opening the browser takes focus from the WebView, and a
 * clipboard write from an unfocused document is refused.
 */
export async function copyHandle(handle: string, addInGame: boolean): Promise<boolean> {
  let copy: Promise<void>;
  try {
    copy = navigator.clipboard.writeText(handle);
  } catch (e) {
    copy = Promise.reject(e);
  }
  if (addInGame) {
    try {
      await openShell(RSI_FRIENDS_URL);
    } catch {
      // No default browser handler: the handle is still copied.
    }
  }
  try {
    await copy;
    return true;
  } catch {
    return false;
  }
}

export function CopyButtons({ handle, verified }: { handle: string; verified: boolean }) {
  const [said, setSaid] = useState('');
  useEffect(() => {
    if (!said) return;
    const t = window.setTimeout(() => setSaid(''), 2500);
    return () => window.clearTimeout(t);
  }, [said]);
  const title = verified ? undefined : 'Handle not verified with RSI';
  const run = async (addInGame: boolean) => {
    const ok = await copyHandle(handle, addInGame);
    setSaid(
      !ok
        ? 'Copy failed'
        : addInGame
          ? "Handle copied — paste it into RSI's add-friend box"
          : 'Handle copied',
    );
  };
  return (
    <span style={{ display: 'inline-flex', gap: 6, alignItems: 'center', flexWrap: 'wrap' }}>
      <GhostButton type="button" disabled={!verified} title={title} onClick={() => void run(false)}>
        Copy handle
      </GhostButton>
      <GhostButton type="button" disabled={!verified} title={title} onClick={() => void run(true)}>
        Add in game
      </GhostButton>
      <span role="status" aria-live="polite" style={{ fontSize: 11, color: 'var(--fg-dim)' }}>
        {said}
      </span>
    </span>
  );
}

const rowStyle = {
  display: 'flex',
  alignItems: 'center',
  justifyContent: 'space-between',
  gap: 8,
  flexWrap: 'wrap' as const,
  padding: '6px 0',
  borderTop: '1px solid var(--border)',
};

export function SocialPane({ webOrigin = null }: { webOrigin?: string | null } = {}) {
  const [friends, setFriends] = useState<FriendsResponse | null>(null);
  const [notes, setNotes] = useState<NotificationsResponse | null>(null);
  const [blocks, setBlocks] = useState<ListedHandle[]>([]);
  const [mutes, setMutes] = useState<ListedHandle[]>([]);
  const [prefs, setPrefs] = useState<SocialPrefs | null>(null);
  const [presence, setPresence] = useState<Record<string, FriendPresence>>({});
  const [level, setLevel] = useState<PresenceLevel | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  const [handle, setHandle] = useState('');

  const refresh = useCallback(async () => {
    // Each call settles on its own so one failure blanks one card.
    const [f, n, b, m, p, pr, lv] = await Promise.allSettled([
      api.socialGetFriends(),
      api.socialGetNotifications(30),
      api.socialGetBlocks(),
      api.socialGetMutes(),
      api.socialGetPrefs(),
      api.socialGetPresence(),
      api.socialGetPresenceLevel(),
    ]);
    if (pr.status === 'fulfilled') {
      setPresence(Object.fromEntries(pr.value.map((x) => [x.handle.toLowerCase(), x])));
    }
    if (lv.status === 'fulfilled') setLevel(lv.value);
    if (f.status === 'fulfilled') setFriends(f.value);
    if (n.status === 'fulfilled') setNotes(n.value);
    if (b.status === 'fulfilled') setBlocks(b.value.blocks);
    if (m.status === 'fulfilled') setMutes(m.value.mutes);
    if (p.status === 'fulfilled') setPrefs(p.value);
    // Only ever SETS an error. Actions refresh after running, and clearing
    // here would wipe the action's own error the moment it was shown.
    const firstFail = [f, n, b, m].find((r) => r.status === 'rejected');
    if (firstFail && firstFail.status === 'rejected') setError(describeError(firstFail.reason));
  }, []);

  useEffect(() => {
    void refresh();
  }, [refresh]);

  // The presence worker emits each friend's presence as the gateway pushes it.
  useEffect(() => {
    let unlisten: (() => void) | undefined;
    let cancelled = false;
    listen<FriendPresence>('friend-presence', (e) => {
      setPresence((prev) => ({ ...prev, [e.payload.handle.toLowerCase()]: e.payload }));
    }).then((unl) => {
      if (cancelled) unl();
      else unlisten = unl;
    });
    return () => {
      cancelled = true;
      unlisten?.();
    };
  }, []);

  // The Rust poller emits after every poll; a changed unread count means a
  // new request or reply, so reload what the pane shows.
  useEffect(() => {
    let unlisten: (() => void) | undefined;
    let cancelled = false;
    let last = -1;
    listen<{ unread_count: number }>('social-notifications', (e) => {
      if (e.payload.unread_count !== last) {
        last = e.payload.unread_count;
        void refresh();
      }
    }).then((unl) => {
      if (cancelled) unl();
      else unlisten = unl;
    });
    return () => {
      cancelled = true;
      unlisten?.();
    };
  }, [refresh]);

  const act = async (fn: () => Promise<unknown>, done: string) => {
    setNotice(null);
    setError(null);
    try {
      await fn();
      setNotice(done);
    } catch (e) {
      setError(describeError(e));
    }
    await refresh();
  };

  // Player lookup as you type: three handle characters, a pause, then the
  // server's matches. Errors just clear the list; the exact-handle send
  // below still works.
  const [matches, setMatches] = useState<string[] | null>(null);
  useEffect(() => {
    const q = handle.trim().replace(/^@/, '');
    if (q.length < 3 || !/^[A-Za-z0-9_-]+$/.test(q)) {
      setMatches(null);
      return;
    }
    let cancelled = false;
    const t = window.setTimeout(() => {
      api
        .socialSearchPlayers(q)
        .then((r) => {
          if (!cancelled) setMatches(r.players.map((p) => p.handle));
        })
        .catch(() => {
          if (!cancelled) setMatches(null);
        });
    }, 300);
    return () => {
      cancelled = true;
      window.clearTimeout(t);
    };
  }, [handle]);

  const onSend = async (ev: FormEvent) => {
    ev.preventDefault();
    await sendTo(handle.trim());
  };

  const sendTo = async (h: string) => {
    if (!h) return;
    setNotice(null);
    setError(null);
    try {
      const r = await api.socialSendRequest(h);
      setNotice(
        r.outcome === 'became_friends'
          ? `@${h} had already asked — you are now friends.`
          : `Request sent to @${h}.`,
      );
      setHandle('');
    } catch (e) {
      setError(describeError(e));
    }
    await refresh();
  };

  const confirmThen = (question: string, fn: () => Promise<unknown>, done: string) => {
    if (window.confirm(question)) void act(fn, done);
  };

  const unread = notes?.unread_count ?? 0;

  return (
    <div style={{ display: 'flex', flexDirection: 'column', gap: 12 }}>
      {error ? <Banner tone="danger">{error}</Banner> : null}
      {notice ? <Banner tone="info">{notice}</Banner> : null}
      <ChatLinkCard webOrigin={webOrigin} />

      <TrayCard
        title="Notifications"
        kicker={unread > 0 ? `${unread} unread` : undefined}
        right={
          unread > 0 ? (
            <GhostButton
              type="button"
              onClick={() => void act(() => api.socialMarkRead([], true), 'All marked read.')}
            >
              Mark all read
            </GhostButton>
          ) : undefined
        }
      >
        {!notes || notes.items.length === 0 ? (
          <p style={{ color: 'var(--fg-dim)', margin: 0 }}>Nothing yet.</p>
        ) : (
          notes.items.map((n) => (
            <NotificationRow
              key={n.id}
              n={n}
              onAccept={(id) => void act(() => api.socialRespond(id, 'accept'), 'Accepted — you are now friends.')}
            />
          ))
        )}
      </TrayCard>

      <TrayCard title="Add a friend">
        <form onSubmit={(e) => void onSend(e)} style={{ display: 'flex', gap: 8 }}>
          <TextInput
            aria-label="StarStats handle"
            placeholder="StarStats handle"
            value={handle}
            maxLength={64}
            onChange={(e) => setHandle(e.target.value)}
          />
          <PrimaryButton type="submit" disabled={!handle.trim()}>
            Send request
          </PrimaryButton>
        </form>
        {matches ? (
          <div data-testid="player-matches" style={{ marginTop: 6 }}>
            {matches.length === 0 ? (
              <p style={{ color: 'var(--fg-dim)', fontSize: 11, margin: 0 }}>
                Nobody found. Only players with a verified RSI handle appear; if you know their
                exact handle, send the request anyway.
              </p>
            ) : (
              matches.map((h) => {
                const lower = h.toLowerCase();
                const isFriend = friends?.friends.some((f) => f.handle.toLowerCase() === lower);
                const asked = friends?.outgoing.some((r) => r.recipient_handle.toLowerCase() === lower);
                return (
                  <div key={h} style={rowStyle} data-testid="player-match">
                    <span>@{h}</span>
                    {isFriend ? (
                      <span style={{ color: 'var(--fg-dim)', fontSize: 11 }}>friends</span>
                    ) : asked ? (
                      <span style={{ color: 'var(--fg-dim)', fontSize: 11 }}>requested</span>
                    ) : (
                      <GhostButton type="button" onClick={() => void sendTo(h)}>
                        Add
                      </GhostButton>
                    )}
                  </div>
                );
              })
            )}
          </div>
        ) : null}
        <p style={{ color: 'var(--fg-dim)', fontSize: 11, margin: '6px 0 0' }}>
          Type three letters of a handle to look players up. Being friends shares no stats.
          Sharing stays a separate choice on the web.
        </p>
      </TrayCard>

      {friends && friends.incoming.length > 0 ? (
        <TrayCard title="Requests" kicker={`${friends.incoming.length} waiting`}>
          {friends.incoming.map((r) => (
            <div key={r.id} style={rowStyle}>
              <span>@{r.requester_handle}</span>
              <span style={{ display: 'inline-flex', gap: 6 }}>
                <PrimaryButton
                  type="button"
                  onClick={() => void act(() => api.socialRespond(r.id, 'accept'), `You and @${r.requester_handle} are now friends.`)}
                >
                  Accept
                </PrimaryButton>
                <GhostButton
                  type="button"
                  onClick={() => void act(() => api.socialRespond(r.id, 'decline'), 'Declined. They are not told.')}
                >
                  Decline
                </GhostButton>
                <DangerButton
                  type="button"
                  onClick={() =>
                    confirmThen(
                      `Block @${r.requester_handle}? They are not told.`,
                      () => api.socialSetBlocked(r.requester_handle, true),
                      'Blocked.',
                    )
                  }
                >
                  Block
                </DangerButton>
              </span>
            </div>
          ))}
        </TrayCard>
      ) : null}

      <TrayCard title="Friends" kicker={friends ? String(friends.friends.length) : undefined}>
        {!friends || friends.friends.length === 0 ? (
          <p style={{ color: 'var(--fg-dim)', margin: 0 }}>No friends yet.</p>
        ) : (
          friends.friends.map((f) => (
            <div key={f.handle} style={rowStyle} data-testid="friend-row">
              <span>
                @{f.handle}
                {f.rsi_verified ? null : (
                  <span style={{ color: 'var(--fg-dim)', fontSize: 11 }}> · not verified</span>
                )}
                {(() => {
                  const label = presenceLabel(presence[f.handle.toLowerCase()]);
                  return label ? (
                    <span data-testid="friend-presence" style={{ color: 'var(--ok, var(--accent))', fontSize: 11 }}>
                      {' '}· {label}
                    </span>
                  ) : null;
                })()}
              </span>
              <span style={{ display: 'inline-flex', gap: 6, flexWrap: 'wrap' }}>
                <CopyButtons handle={f.handle} verified={f.rsi_verified} />
                <GhostButton
                  type="button"
                  onClick={() => void act(() => api.socialSetMuted(f.handle, true), `Muted @${f.handle}.`)}
                >
                  Mute
                </GhostButton>
                <GhostButton
                  type="button"
                  onClick={() =>
                    confirmThen(
                      `Remove @${f.handle} from your friends?`,
                      () => api.socialRemoveFriend(f.handle),
                      `Removed @${f.handle}.`,
                    )
                  }
                >
                  Unfriend
                </GhostButton>
                <DangerButton
                  type="button"
                  onClick={() =>
                    confirmThen(
                      `Block @${f.handle}? This also unfriends them and revokes any stats share you gave them.`,
                      () => api.socialSetBlocked(f.handle, true),
                      `Blocked @${f.handle}.`,
                    )
                  }
                >
                  Block
                </DangerButton>
              </span>
            </div>
          ))
        )}
      </TrayCard>

      {friends && friends.outgoing.length > 0 ? (
        <TrayCard title="Sent requests">
          {friends.outgoing.map((r) => (
            <div key={r.id} style={rowStyle}>
              <span>@{r.recipient_handle}</span>
              <GhostButton
                type="button"
                onClick={() => void act(() => api.socialRespond(r.id, 'cancel'), 'Request withdrawn.')}
              >
                Withdraw
              </GhostButton>
            </div>
          ))}
        </TrayCard>
      ) : null}

      <TrayCard title="Privacy">
        {friends ? (
          <label style={{ display: 'flex', gap: 8, alignItems: 'center' }}>
            Friend requests from
            <select
              value={friends.friend_request_policy}
              onChange={(e) => {
                const v = e.target.value as FriendRequestPolicy;
                void act(() => api.socialUpdateSettings({ friend_request_policy: v }), 'Saved.');
              }}
            >
              <option value="everyone">anyone</option>
              <option value="org_mates">people in one of my orgs</option>
              <option value="nobody">nobody</option>
            </select>
          </label>
        ) : null}
        {friends ? (
          <label style={{ display: 'flex', gap: 8, alignItems: 'center', marginTop: 6 }}>
            <input
              type="checkbox"
              checked={friends.discoverable}
              onChange={(e) => {
                const on = e.target.checked;
                void act(
                  () => api.socialUpdateSettings({ discoverable: on }),
                  on ? 'Players can find you by your handle.' : 'Players can no longer look you up.',
                );
              }}
            />
            Let players find me by my handle
          </label>
        ) : null}
        <HandleRows
          title="Blocked"
          rows={blocks}
          button="Unblock"
          onClick={(h) => void act(() => api.socialSetBlocked(h, false), `Unblocked @${h}.`)}
        />
        <HandleRows
          title="Muted"
          rows={mutes}
          button="Unmute"
          onClick={(h) => void act(() => api.socialSetMuted(h, false), `Unmuted @${h}.`)}
        />
      </TrayCard>

      {prefs ? (
        <TrayCard title="Presence">
          <p style={{ color: 'var(--fg-dim)', margin: '0 0 8px', fontSize: 12 }}>
            Friends can see whether you are online, in game or in quantum, and your star
            system if you allow it. Both switches below must be on. Nothing is kept once
            you go offline.
          </p>
          <label style={{ display: 'flex', gap: 8, alignItems: 'center' }}>
            <input
              type="checkbox"
              checked={prefs.share_presence}
              onChange={(e) => {
                const next = { ...prefs, share_presence: e.target.checked };
                void act(() => api.socialSetPrefs(next), 'Saved.');
              }}
            />
            Share my presence from this tray
          </label>
          {level ? (
            <label style={{ display: 'flex', gap: 8, alignItems: 'center', marginTop: 6 }}>
              Friends see
              <select
                value={level}
                onChange={(e) => {
                  const v = e.target.value as PresenceLevel;
                  void act(async () => {
                    setLevel(await api.socialSetPresenceLevel(v));
                  }, 'Saved.');
                }}
              >
                <option value="off">nothing</option>
                <option value="status">whether I am online, in game or in quantum</option>
                <option value="system">that, and my star system</option>
              </select>
            </label>
          ) : null}
        </TrayCard>
      ) : null}

      {prefs ? (
        <TrayCard title="Desktop notifications">
          <label style={{ display: 'flex', gap: 8, alignItems: 'center' }}>
            <input
              type="checkbox"
              checked={prefs.toasts}
              onChange={(e) => {
                const next = { ...prefs, toasts: e.target.checked };
                void act(() => api.socialSetPrefs(next), 'Saved.');
              }}
            />
            Show a desktop notification for friend requests and replies
          </label>
          <label style={{ display: 'flex', gap: 8, alignItems: 'center', marginTop: 6 }}>
            <input
              type="checkbox"
              checked={prefs.quiet_in_game}
              disabled={!prefs.toasts}
              onChange={(e) => {
                const next = { ...prefs, quiet_in_game: e.target.checked };
                void act(() => api.socialSetPrefs(next), 'Saved.');
              }}
            />
            Hold them while Star Citizen is running
          </label>
          <label style={{ display: 'flex', gap: 8, alignItems: 'center', marginTop: 6 }}>
            <input
              type="checkbox"
              checked={prefs.release_toasts}
              onChange={(e) => {
                const next = { ...prefs, release_toasts: e.target.checked };
                void act(() => api.socialSetPrefs(next), 'Saved.');
              }}
            />
            Tell me about new features and StarStats updates
          </label>
        </TrayCard>
      ) : null}
    </div>
  );
}

function NotificationRow({
  n,
  onAccept,
}: {
  n: SocialNotification;
  onAccept: (requestId: string) => void;
}) {
  const who = n.actor_handle ?? 'Someone';
  const text =
    n.kind === 'friend_request'
      ? `@${who} sent you a friend request`
      : n.kind === 'friend_accepted'
        ? `@${who} accepted your friend request`
        : n.kind === 'salute'
          ? `@${who} saluted your profile. o7`
          : n.kind === 'lfg_join_request'
            ? `@${who} asked to join your group`
            : n.kind === 'lfg_join_accepted'
              ? `@${who} accepted you into their group`
              : n.kind === 'commend'
                ? // Anonymous by design: the word, never who gave it.
                  typeof n.payload.kind === 'string'
                  ? `A crewmate commended you: ${commendLabel(n.payload.kind)}`
                  : 'A crewmate commended you'
                : `New activity from @${who}`;
  return (
    <div style={rowStyle} data-unread={n.read_at ? undefined : 'true'}>
      <span style={{ color: n.read_at ? 'var(--fg-muted)' : 'var(--fg)' }}>
        {text}
        <span style={{ color: 'var(--fg-dim)', fontSize: 11 }}> · {relativeTimeSince(n.created_at)}</span>
      </span>
      <span style={{ display: 'inline-flex', gap: 6, flexWrap: 'wrap' }}>
        {n.kind === 'friend_request' && n.payload.request_id ? (
          <PrimaryButton type="button" onClick={() => onAccept(n.payload.request_id as string)}>
            Accept
          </PrimaryButton>
        ) : null}
        {n.actor_handle ? (
          <CopyButtons handle={n.actor_handle} verified={n.payload.rsi_verified === true} />
        ) : null}
      </span>
    </div>
  );
}

function HandleRows({
  title,
  rows,
  button,
  onClick,
}: {
  title: string;
  rows: ListedHandle[];
  button: string;
  onClick: (handle: string) => void;
}) {
  return (
    <div style={{ marginTop: 10 }}>
      <div style={{ fontSize: 11, color: 'var(--fg-dim)', textTransform: 'uppercase' }}>{title}</div>
      {rows.length === 0 ? (
        <p style={{ color: 'var(--fg-dim)', margin: '4px 0 0' }}>None.</p>
      ) : (
        rows.map((r) => (
          <div key={r.handle} style={rowStyle}>
            <span>@{r.handle}</span>
            <GhostButton type="button" onClick={() => onClick(r.handle)}>
              {button}
            </GhostButton>
          </div>
        ))
      )}
    </div>
  );
}
