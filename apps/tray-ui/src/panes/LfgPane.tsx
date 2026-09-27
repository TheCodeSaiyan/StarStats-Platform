/**
 * Crew: the Looking for Group board in the tray.
 *
 * The same board as the web's /lfg, with the one thing only the tray can
 * do: "Fill from the game" pre-fills a post with the star system, the
 * last place and the ship from the game log. Every rule (verified handle,
 * one open post, blocks, capacity) is the server's; its error codes arrive
 * as the rejection and are turned into copy here.
 */

import { useCallback, useEffect, useState, type FormEvent } from 'react';
import { listen } from '@tauri-apps/api/event';
import {
  api,
  type CrewOverview,
  type LfgOptions,
  type LfgPost,
  type LfgPostDetail,
  type NewLfgPost,
} from '../api';
import {
  Banner,
  DangerButton,
  Field,
  GhostButton,
  PrimaryButton,
  TextInput,
  TrayCard,
} from '../components/tray/primitives';
import { COMMEND_KINDS, commendLabel } from '../lib/commends';
import { activityLabel, crewHandles, matchSystem, regionLabel, voiceLabel } from '../lib/lfg';
import { CopyButtons } from './SocialPane';

const ERROR_COPY: Record<string, string> = {
  rsi_handle_not_verified:
    'Verify your RSI handle first (on the web: Calibrate → RSI handle). Crew add each other in game by handle.',
  invalid_system: 'Pick a star system from the list.',
  invalid_crew_slots: 'Crew must be between 1 and 30.',
  already_posting: 'You already have an open post. Close it to post another.',
  rate_limited: 'That is a lot for one day. Try again tomorrow.',
  not_found: 'That post has ended or is not available.',
  own_post: 'That is your own post.',
  already_asked: 'You have already asked to join that group.',
  group_full: 'That group is full.',
  removed_from_group: 'The host removed you from that group.',
  post_ended: 'That post has ended.',
  account_restricted: 'Your account is restricted from this for now.',
  post_not_ended: 'You can commend your crew once the post has ended.',
  window_closed: 'Commends for that group have closed. They stay open for 48 hours.',
};

function describe(e: unknown): string {
  const msg = String(e);
  if (msg.includes('not paired')) return 'Pair this tray to use the board.';
  return ERROR_COPY[msg] ?? msg;
}

const selectStyle = {
  background: 'var(--bg-elev)',
  color: 'var(--fg)',
  border: '1px solid var(--border)',
  padding: '4px 6px',
};

const rowStyle = {
  display: 'flex',
  alignItems: 'center',
  justifyContent: 'space-between',
  gap: 8,
  flexWrap: 'wrap' as const,
  padding: '6px 0',
  borderTop: '1px solid var(--border)',
};

function emptyForm(o: LfgOptions | null): NewLfgPost {
  return {
    activity: o?.activities[0] ?? 'mining',
    system: null,
    location: null,
    ship: null,
    crew_slots: 2,
    voice: 'optional',
    region: 'any',
    note: null,
    expires_in_minutes: o?.expiry_default_minutes ?? 120,
  };
}

export function LfgPane() {
  const [options, setOptions] = useState<LfgOptions | null>(null);
  const [posts, setPosts] = useState<LfgPost[] | null>(null);
  const [mine, setMine] = useState<LfgPostDetail | null>(null);
  const [crew, setCrew] = useState<CrewOverview | null>(null);
  const [form, setForm] = useState<NewLfgPost>(emptyForm(null));
  const [error, setError] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);

  const refresh = useCallback(async () => {
    const [o, l, c] = await Promise.allSettled([
      api.lfgOptions(),
      api.lfgList(null, null),
      api.crewMine(),
    ]);
    if (o.status === 'fulfilled') setOptions(o.value);
    // Informational: an older server or a blip hides the crew cards only.
    if (c.status === 'fulfilled') setCrew(c.value);
    if (l.status === 'fulfilled') {
      setPosts(l.value.posts);
      const own = l.value.posts.find((p) => p.is_host);
      setMine(own ? await api.lfgGet(own.id).catch(() => null) : null);
    } else {
      setError(describe(l.reason));
    }
  }, []);

  useEffect(() => {
    void refresh();
    // Posts expire within hours; a minute keeps the board honest without
    // hammering the server.
    const t = window.setInterval(() => void refresh(), 60_000);
    return () => window.clearInterval(t);
  }, [refresh]);

  // A join request or an acceptance arrives as a notification; reload.
  useEffect(() => {
    let unlisten: (() => void) | undefined;
    let cancelled = false;
    listen('social-notifications', () => void refresh()).then((unl) => {
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
      setError(describe(e));
    }
    await refresh();
  };

  const fillFromGame = async () => {
    try {
      const w = await api.lfgWhereAmI();
      setForm((f) => ({
        ...f,
        system: matchSystem(options?.systems ?? [], w.system) || f.system,
        location: w.location ?? f.location,
        ship: w.ship ?? f.ship,
      }));
      setNotice(
        w.system || w.location || w.ship
          ? 'Filled from your game log. Change anything that is wrong.'
          : 'Nothing to fill yet: play a little first.',
      );
    } catch (e) {
      setError(describe(e));
    }
  };

  const onPost = async (ev: FormEvent) => {
    ev.preventDefault();
    await act(async () => {
      await api.lfgCreate({
        ...form,
        system: form.system || null,
        location: form.location?.trim() || null,
        ship: form.ship?.trim() || null,
        note: form.note?.trim() || null,
      });
      setForm(emptyForm(options));
    }, 'Posted. It stays up until it expires, you close it, or you have been out of the game for 10 minutes.');
  };

  const copyCrew = async () => {
    if (!mine) return;
    try {
      await navigator.clipboard.writeText(crewHandles(mine.members));
      setNotice('Crew handles copied, one per line.');
    } catch {
      setError('Copy failed.');
    }
  };

  const others = (posts ?? []).filter((p) => !p.is_host);

  return (
    <div style={{ display: 'flex', flexDirection: 'column', gap: 12 }}>
      {error ? <Banner tone="danger">{error}</Banner> : null}
      {notice ? <Banner tone="info">{notice}</Banner> : null}

      {mine ? (
        <TrayCard title="Your post" kicker={`${mine.crew_count}/${mine.crew_slots} crew`}>
          <p style={{ margin: '0 0 6px', color: 'var(--fg-dim)' }}>
            {activityLabel(mine.activity)}
            {mine.system ? ` · ${mine.system}` : ''}
            {mine.ship ? ` · ${mine.ship}` : ''}
          </p>
          {mine.members
            .filter((m) => m.status === 'requested' || m.status === 'accepted')
            .map((m) => (
              <div key={m.handle} style={rowStyle} data-testid="lfg-member">
                <span>
                  @{m.handle}
                  <span style={{ color: 'var(--fg-dim)', fontSize: 11 }}>
                    {m.status === 'requested' ? ' · asking' : ' · crew'}
                  </span>
                </span>
                <span style={{ display: 'inline-flex', gap: 6, flexWrap: 'wrap' }}>
                  {m.status === 'requested' ? (
                    <>
                      <PrimaryButton
                        type="button"
                        onClick={() => void act(() => api.lfgRespond(mine.id, m.handle, 'accept'), `@${m.handle} is in.`)}
                      >
                        Accept
                      </PrimaryButton>
                      <GhostButton
                        type="button"
                        onClick={() => void act(() => api.lfgRespond(mine.id, m.handle, 'decline'), 'Declined.')}
                      >
                        Decline
                      </GhostButton>
                    </>
                  ) : (
                    <>
                      <CopyButtons handle={m.handle} verified />
                      <GhostButton
                        type="button"
                        onClick={() => {
                          if (window.confirm(`Remove @${m.handle}? They cannot ask again.`)) {
                            void act(() => api.lfgRespond(mine.id, m.handle, 'remove'), 'Removed.');
                          }
                        }}
                      >
                        Remove
                      </GhostButton>
                    </>
                  )}
                </span>
              </div>
            ))}
          <span style={{ display: 'inline-flex', gap: 6, marginTop: 8 }}>
            <GhostButton type="button" onClick={() => void copyCrew()} disabled={mine.crew_count === 0}>
              Copy all crew handles
            </GhostButton>
            <DangerButton
              type="button"
              onClick={() => {
                if (window.confirm('Close your post? It leaves the board.')) {
                  void act(() => api.lfgClose(mine.id), 'Your post is closed.');
                }
              }}
            >
              Close post
            </DangerButton>
          </span>
        </TrayCard>
      ) : null}

      {crew && crew.windows.length > 0 ? (
        <TrayCard title="Commend your crew" kicker={`${crew.windows.length} open`}>
          <p style={{ margin: '0 0 6px', color: 'var(--fg-dim)', fontSize: 12 }}>
            One word each, for someone you just flew with. Nobody sees who gave which, not even
            them, though in a crew of two they can work it out.
          </p>
          {crew.windows.map((w) => (
            <div key={w.post_id} data-testid="commend-window">
              <p style={{ margin: '6px 0 0', color: 'var(--fg-dim)', fontSize: 11 }}>
                {activityLabel(w.activity)} · open until{' '}
                {new Date(w.closes_at).toLocaleString(undefined, {
                  weekday: 'short',
                  hour: '2-digit',
                  minute: '2-digit',
                })}
              </p>
              {w.crew.map((m) => (
                <div key={m.handle} style={rowStyle} data-testid="commend-mate">
                  <span>@{m.handle}</span>
                  <span
                    style={{ display: 'inline-flex', gap: 6, flexWrap: 'wrap' }}
                    role="group"
                    aria-label={`Commend @${m.handle}`}
                  >
                    {COMMEND_KINDS.map((kind) =>
                      m.my_commend === kind ? (
                        <PrimaryButton
                          key={kind}
                          type="button"
                          aria-pressed
                          onClick={() =>
                            void act(
                              () => api.crewWithdrawCommend(w.post_id, m.handle),
                              'Commend withdrawn.',
                            )
                          }
                        >
                          {commendLabel(kind)}
                        </PrimaryButton>
                      ) : (
                        <GhostButton
                          key={kind}
                          type="button"
                          aria-pressed={false}
                          onClick={() =>
                            void act(
                              () => api.crewCommend(w.post_id, m.handle, kind),
                              `Commended @${m.handle}: ${commendLabel(kind)}.`,
                            )
                          }
                        >
                          {commendLabel(kind)}
                        </GhostButton>
                      ),
                    )}
                  </span>
                </div>
              ))}
            </div>
          ))}
        </TrayCard>
      ) : null}

      <TrayCard title="Looking for Group" kicker={posts ? `${others.length} open` : undefined}>
        {!posts ? (
          <p style={{ color: 'var(--fg-dim)', margin: 0 }}>Loading…</p>
        ) : others.length === 0 ? (
          <p style={{ color: 'var(--fg-dim)', margin: 0 }}>Nobody is looking right now.</p>
        ) : (
          others.map((p) => {
            const full = p.crew_count >= p.crew_slots;
            return (
              <div key={p.id} style={rowStyle} data-testid="lfg-post">
                <span>
                  {activityLabel(p.activity)} with @{p.host_handle}
                  <span style={{ color: 'var(--fg-dim)', fontSize: 11, display: 'block' }}>
                    {[p.system, p.location, p.ship, `${p.crew_count}/${p.crew_slots} crew`, voiceLabel(p.voice), regionLabel(p.region)]
                      .filter(Boolean)
                      .join(' · ')}
                  </span>
                  {p.note ? (
                    <span style={{ color: 'var(--fg-dim)', fontSize: 11, display: 'block' }}>“{p.note}”</span>
                  ) : null}
                </span>
                <span style={{ display: 'inline-flex', gap: 6, flexWrap: 'wrap' }}>
                  {p.my_status === 'accepted' ? (
                    <>
                      <CopyButtons handle={p.host_handle} verified={p.host_verified} />
                      <GhostButton type="button" onClick={() => void act(() => api.lfgLeave(p.id), 'You left the group.')}>
                        Leave
                      </GhostButton>
                    </>
                  ) : p.my_status === 'requested' ? (
                    <GhostButton type="button" onClick={() => void act(() => api.lfgLeave(p.id), 'Request withdrawn.')}>
                      Withdraw
                    </GhostButton>
                  ) : p.my_status === 'removed' ? null : (
                    <PrimaryButton
                      type="button"
                      disabled={full}
                      onClick={() => void act(() => api.lfgJoin(p.id), 'Asked. You will be told if the host accepts.')}
                    >
                      {full ? 'Full' : 'Ask to join'}
                    </PrimaryButton>
                  )}
                </span>
              </div>
            );
          })
        )}
      </TrayCard>

      {crew && crew.history.length > 0 ? (
        <TrayCard title="Players you flew with" kicker="last 90 days">
          <p style={{ margin: '0 0 6px', color: 'var(--fg-dim)', fontSize: 12 }}>
            Only you can see this list.
          </p>
          {crew.history.slice(0, 20).map((m) => (
            <div key={`${m.post_id}-${m.handle}`} style={rowStyle} data-testid="crew-history">
              <span>
                @{m.handle}
                <span style={{ color: 'var(--fg-dim)', fontSize: 11 }}>
                  {' '}
                  · {activityLabel(m.activity)} ·{' '}
                  {new Date(m.flew_at).toLocaleDateString(undefined, { day: 'numeric', month: 'short' })}
                </span>
              </span>
              <CopyButtons handle={m.handle} verified />
            </div>
          ))}
        </TrayCard>
      ) : null}

      {!mine && options ? (
        <TrayCard title="Post a group">
          <form onSubmit={(e) => void onPost(e)} style={{ display: 'flex', flexDirection: 'column', gap: 6 }}>
            <GhostButton type="button" onClick={() => void fillFromGame()}>
              Fill from the game
            </GhostButton>
            <Field label="Activity">
              <select
                style={selectStyle}
                value={form.activity}
                onChange={(e) => setForm({ ...form, activity: e.target.value })}
              >
                {options.activities.map((a) => (
                  <option key={a} value={a}>
                    {activityLabel(a)}
                  </option>
                ))}
              </select>
            </Field>
            <Field label="System">
              <select
                style={selectStyle}
                value={form.system ?? ''}
                onChange={(e) => setForm({ ...form, system: e.target.value || null })}
              >
                <option value="">Not saying</option>
                {options.systems.map((s) => (
                  <option key={s} value={s}>
                    {s}
                  </option>
                ))}
              </select>
            </Field>
            <Field label="Where to meet">
              <TextInput
                maxLength={64}
                value={form.location ?? ''}
                onChange={(e) => setForm({ ...form, location: e.target.value })}
              />
            </Field>
            <Field label="Ship">
              <TextInput
                maxLength={64}
                value={form.ship ?? ''}
                onChange={(e) => setForm({ ...form, ship: e.target.value })}
              />
            </Field>
            <Field label="Crew wanted">
              <TextInput
                type="number"
                min={options.crew_min}
                max={options.crew_max}
                value={form.crew_slots}
                onChange={(e) => setForm({ ...form, crew_slots: Number(e.target.value) })}
              />
            </Field>
            <Field label="Voice">
              <select style={selectStyle} value={form.voice} onChange={(e) => setForm({ ...form, voice: e.target.value })}>
                {options.voices.map((v) => (
                  <option key={v} value={v}>
                    {voiceLabel(v)}
                  </option>
                ))}
              </select>
            </Field>
            <Field label="Region">
              <select style={selectStyle} value={form.region} onChange={(e) => setForm({ ...form, region: e.target.value })}>
                {options.regions.map((r) => (
                  <option key={r} value={r}>
                    {regionLabel(r)}
                  </option>
                ))}
              </select>
            </Field>
            <Field
              label="Stays up for"
              hint="Or until 10 minutes after you leave the game, if you posted while playing."
            >
              <select
                style={selectStyle}
                value={form.expires_in_minutes}
                onChange={(e) => setForm({ ...form, expires_in_minutes: Number(e.target.value) })}
              >
                {[30, 60, 120, 240, 360]
                  .filter((m) => m >= options.expiry_min_minutes && m <= options.expiry_max_minutes)
                  .map((m) => (
                    <option key={m} value={m}>
                      {m < 60 ? `${m} minutes` : `${m / 60} ${m === 60 ? 'hour' : 'hours'}`}
                    </option>
                  ))}
              </select>
            </Field>
            <Field label="Note" hint="Optional, 200 characters. Everyone on the board sees it.">
              <TextInput
                maxLength={200}
                value={form.note ?? ''}
                onChange={(e) => setForm({ ...form, note: e.target.value })}
              />
            </Field>
            <PrimaryButton type="submit">Post</PrimaryButton>
          </form>
        </TrayCard>
      ) : null}
    </div>
  );
}
