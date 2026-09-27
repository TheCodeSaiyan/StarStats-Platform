/**
 * Looking for Group: a global board of short-lived calls for crew.
 *
 * Backend contracts:
 *  - GET  /v1/lfg/options                        — vocabularies and limits
 *  - GET  /v1/lfg?activity=&system=              — open posts
 *  - POST /v1/lfg                                — post (verified handle)
 *  - GET  /v1/lfg/:id                            — a post, with its crew for the host
 *  - DEL  /v1/lfg/:id                            — the host closes it
 *  - POST/DEL /v1/lfg/:id/join                   — ask to join / leave
 *  - PUT  /v1/lfg/:id/members/:handle            — accept | decline | remove
 *  - POST /v1/lfg/:id/report                     — report to moderators
 *  - GET  /v1/me/crew                            — crew history, commend windows
 *  - PUT/DEL /v1/crew/:post/commends/:handle     — commend a crewmate
 *
 * Each section is fed by its own call through `allSettled`, so one failing
 * endpoint blanks one section, not the page.
 */
import { redirect } from 'next/navigation';
import React from 'react';
import { BeamAlert, BeamButton, BeamChip, BeamInput, BeamSelect, BeamTextarea, Plane } from 'holo';
import type { Calibration } from 'holo';
import {
  ApiCallError,
  getLfgOptions,
  getLfgPost,
  getMyCrew,
  listLfgPosts,
  type CommendWindow,
  type CrewMate,
  type CrewOverview,
  type LfgOptions,
  type LfgPostDetail,
  type LfgPostView,
} from '@/lib/api';
import { COMMEND_KINDS, commendLabel } from '@/lib/commends';
import { activityLabel, regionLabel, timeLeft, voiceLabel } from '@/lib/lfg';
import { logger } from '@/lib/logger';
import { navSections } from '@/lib/nav';
import { getSession } from '@/lib/session';
import { getTheme } from '@/lib/theme';
import { setCalibrationAction } from '@/app/me/_projection/actions';
import { ConfirmSubmitButton } from '@/components/forms/ConfirmSubmitButton';
import { CopyHandleButton } from '@/components/social/CopyHandleButton';
import { LfgProjection, type LfgSection } from './_projection/LfgProjection';
import {
  closePostAction,
  commendAction,
  createPostAction,
  joinAction,
  leaveAction,
  reportAction,
  respondAction,
} from './actions';

export const metadata = { title: 'Looking for Group' };

const STATUS_MESSAGES: Record<string, string> = {
  posted: 'Posted. It is on the board until it expires or you close it.',
  closed: 'Your post is closed.',
  asked: 'Asked. The host decides; you will get a notification if they accept.',
  left: 'You are no longer in that group.',
  member_accepted: 'Accepted. They have been told, with your handle to add in game.',
  member_declined: 'Declined. They are not told why.',
  member_removed: 'Removed. They cannot ask to join this group again.',
  reported: 'Thanks. A moderator will look at it.',
  commended: 'Commended. They are told the word, not who gave it.',
  commend_withdrawn: 'Commend withdrawn.',
};

const ERROR_MESSAGES: Record<string, string> = {
  rsi_handle_not_verified:
    'Verify your RSI handle first (Calibrate → RSI handle). Crew add each other in game by handle.',
  invalid_system: 'Pick a star system from the list.',
  invalid_crew_slots: 'Crew must be between 1 and 30.',
  invalid_expiry: 'Pick how long the post stays up.',
  location_too_long: 'Location is too long.',
  ship_too_long: 'Ship is too long.',
  note_too_long: 'The note is too long.',
  details_too_long: 'The report details are too long.',
  location_invalid: 'Location has characters it cannot contain.',
  ship_invalid: 'Ship has characters it cannot contain.',
  note_invalid: 'The note has characters it cannot contain.',
  already_posting: 'You already have an open post. Close it to post another.',
  rate_limited: 'That is a lot for one day. Try again tomorrow.',
  not_found: 'That post has ended or is not available.',
  own_post: 'That is your own post.',
  already_asked: 'You have already asked to join that group.',
  group_full: 'That group is full.',
  removed_from_group: 'The host removed you from that group.',
  post_ended: 'That post has ended.',
  not_requested: 'They are not waiting on an answer.',
  account_restricted: 'Your account is restricted from this for now.',
  post_not_ended: 'You can commend your crew once the post has ended.',
  window_closed: 'Commends for that group have closed. They stay open for 48 hours.',
  cannot_commend_self: 'You cannot commend yourself.',
  unexpected: 'Something went wrong. Try again.',
};

type SearchParams = { status?: string; error?: string; activity?: string; system?: string };

export default async function LfgPage(props: { searchParams: Promise<SearchParams> }) {
  const session = await getSession();
  if (!session) redirect('/auth/login?next=/lfg');
  const params = await props.searchParams;

  let calibration: Calibration = 'terra';
  try {
    calibration = (await getTheme(session.token)) as Calibration;
  } catch (e) {
    logger.warn({ err: e, call: 'lfg.theme' }, 'load theme failed');
  }

  const [optionsRes, boardRes, crewRes] = await Promise.allSettled([
    getLfgOptions(),
    listLfgPosts(session.token, { activity: params.activity, system: params.system }),
    getMyCrew(session.token),
  ]);
  for (const [r, call] of [
    [optionsRes, 'lfg.options'],
    [boardRes, 'lfg.list'],
    [crewRes, 'lfg.crew'],
  ] as const) {
    if (r.status === 'rejected') {
      const status = r.reason instanceof ApiCallError ? r.reason.status : undefined;
      if (status === 401) redirect('/auth/login?next=/lfg');
      logger.error({ err: r.reason, call, status }, 'lfg page call failed');
    }
  }
  const options: LfgOptions | null = optionsRes.status === 'fulfilled' ? optionsRes.value : null;
  const board: LfgPostView[] | null =
    boardRes.status === 'fulfilled' ? boardRes.value.posts : null;
  const crew: CrewOverview | null = crewRes.status === 'fulfilled' ? crewRes.value : null;

  // The host's own post, with everyone who asked. The list is filtered, so
  // look for it unfiltered when a filter is on.
  let mine: LfgPostDetail | null = null;
  const minePost =
    board?.find((p) => p.is_host) ??
    (params.activity || params.system
      ? (await listLfgPosts(session.token).catch(() => ({ posts: [] }))).posts.find(
          (p) => p.is_host,
        )
      : undefined);
  if (minePost) {
    try {
      mine = await getLfgPost(session.token, minePost.id);
    } catch (e) {
      logger.warn({ err: e, call: 'lfg.mine' }, 'own post fetch failed');
    }
  }
  const joined = (board ?? []).filter(
    (p) => p.my_status === 'accepted' || p.my_status === 'requested',
  );

  // eslint-disable-next-line react-hooks/purity -- server component: read once per request
  const now = Date.now();
  const unavailable = (what: string) => (
    <BeamAlert tone="bad">Couldn&apos;t load {what}. Refresh to retry.</BeamAlert>
  );

  const sections: LfgSection[] = [
    {
      id: 'board',
      title: 'Looking for Group',
      ctx: board ? `${board.length} open` : undefined,
      group: 'board',
      node: (
        <>
          <p className="hp-prose">
            Posts expire, usually within two hours, so what is here is live. Ask to join, and
            the host will add you in game by handle.
          </p>
          {options ? (
            <form method="get" className="hp-formrow" style={{ flexWrap: 'wrap' }}>
              <BeamSelect id="lfg-filter-activity" name="activity" label="Activity" defaultValue={params.activity ?? ''}>
                <option value="">Any</option>
                {options.activities.map((a) => (
                  <option key={a} value={a}>
                    {activityLabel(a)}
                  </option>
                ))}
              </BeamSelect>
              <BeamSelect id="lfg-filter-system" name="system" label="System" defaultValue={params.system ?? ''}>
                <option value="">Any</option>
                {options.systems.map((s) => (
                  <option key={s} value={s}>
                    {s}
                  </option>
                ))}
              </BeamSelect>
              <BeamButton type="submit">Filter</BeamButton>
            </form>
          ) : null}
          {!board ? (
            unavailable('the board')
          ) : board.length === 0 ? (
            <p className="hp-prose">
              Nobody is looking right now{params.activity || params.system ? ' for that' : ''}.
              Post your own from the Post tab.
            </p>
          ) : (
            <Plane tilt="flat" style={{ marginTop: 18 }}>
              {board.map((p) => (
                <PostRow key={p.id} post={p} now={now} />
              ))}
            </Plane>
          )}
        </>
      ),
    },
    {
      id: 'hosting',
      title: 'Your post',
      group: 'mine',
      node: !mine ? (
        <p className="hp-prose">You have no open post.</p>
      ) : (
        <HostPanel detail={mine} now={now} />
      ),
    },
    {
      id: 'joined',
      title: 'Groups you asked to join',
      group: 'mine',
      node:
        joined.length === 0 ? (
          <p className="hp-prose">None right now.</p>
        ) : (
          <Plane tilt="flat" style={{ marginTop: 18 }}>
            {joined.map((p) => (
              <PostRow key={p.id} post={p} now={now} />
            ))}
          </Plane>
        ),
    },
    {
      id: 'new',
      title: 'Post a group',
      group: 'post',
      node: !options ? (
        unavailable('the post form')
      ) : mine ? (
        <p className="hp-prose">You already have an open post. Close it to post another.</p>
      ) : (
        <form action={createPostAction} className="hp-formcol">
          <BeamSelect id="lfg-activity" name="activity" label="Activity" required defaultValue="mining">
            {options.activities.map((a) => (
              <option key={a} value={a}>
                {activityLabel(a)}
              </option>
            ))}
          </BeamSelect>
          <BeamSelect id="lfg-system" name="system" label="System" defaultValue="">
            <option value="">Not saying</option>
            {options.systems.map((s) => (
              <option key={s} value={s}>
                {s}
              </option>
            ))}
          </BeamSelect>
          <BeamInput id="lfg-location" name="location" label="Where to meet" maxLength={64} hint="Optional. A station, city or landmark." />
          <BeamInput id="lfg-ship" name="ship" label="Ship" maxLength={64} hint="Optional." />
          <BeamInput
            id="lfg-crew"
            name="crew_slots"
            type="number"
            label="Crew wanted"
            min={options.crew_min}
            max={options.crew_max}
            defaultValue={2}
            required
          />
          <BeamSelect id="lfg-voice" name="voice" label="Voice" defaultValue="optional">
            {options.voices.map((v) => (
              <option key={v} value={v}>
                {voiceLabel(v)}
              </option>
            ))}
          </BeamSelect>
          <BeamSelect id="lfg-region" name="region" label="Region" defaultValue="any">
            {options.regions.map((r) => (
              <option key={r} value={r}>
                {regionLabel(r)}
              </option>
            ))}
          </BeamSelect>
          <BeamSelect
            id="lfg-expiry"
            name="expires_in_minutes"
            label="Stays up for"
            defaultValue={String(options.expiry_default_minutes)}
          >
            {[30, 60, 120, 240, 360]
              .filter((m) => m >= options.expiry_min_minutes && m <= options.expiry_max_minutes)
              .map((m) => (
                <option key={m} value={m}>
                  {m < 60 ? `${m} minutes` : `${m / 60} ${m === 60 ? 'hour' : 'hours'}`}
                </option>
              ))}
          </BeamSelect>
          <BeamTextarea
            id="lfg-note"
            name="note"
            label="Note"
            maxLength={200}
            rows={3}
            hint="Optional, 200 characters. Visible to everyone on the board."
          />
          <BeamButton type="submit" variant="primary" style={{ alignSelf: 'flex-start' }}>
            Post
          </BeamButton>
        </form>
      ),
    },
    {
      id: 'commend',
      title: 'Commend your crew',
      ctx: crew && crew.windows.length > 0 ? `${crew.windows.length} open` : undefined,
      group: 'crew',
      node: !crew ? (
        unavailable('your crew')
      ) : crew.windows.length === 0 ? (
        <p className="hp-prose">
          When a group you flew in ends, you have 48 hours to commend your crewmates here.
        </p>
      ) : (
        <>
          <p className="hp-prose">
            One word each, for someone you just flew with. It counts towards the totals on their
            profile. Nobody sees who gave which, not even them, though in a crew of two they can
            work it out.
          </p>
          {crew.windows.map((w) => (
            <CrewWindow key={w.post_id} window={w} now={now} />
          ))}
        </>
      ),
    },
    {
      id: 'history',
      title: 'Players you flew with',
      ctx: crew ? `${crew.history.length}` : undefined,
      group: 'crew',
      node: !crew ? (
        unavailable('your crew history')
      ) : crew.history.length === 0 ? (
        <p className="hp-prose">
          Nobody yet. Players you crew with through Looking for Group appear here for 90 days.
          Only you can see this list.
        </p>
      ) : (
        <>
          <p className="hp-prose">
            The last 90 days, newest first. Only you can see this list.
          </p>
          <CrewHistory history={crew.history} />
        </>
      ),
    },
  ];

  const notice =
    params.status && STATUS_MESSAGES[params.status]
      ? { tone: 'good' as const, message: STATUS_MESSAGES[params.status] }
      : params.error
        ? { tone: 'bad' as const, message: ERROR_MESSAGES[params.error] ?? ERROR_MESSAGES.unexpected }
        : null;

  return (
    <LfgProjection
      handle={session.claimedHandle}
      calibration={calibration}
      nav={navSections({ signedIn: true, staffRoles: session.staffRoles }, 'lfg')}
      sections={sections}
      notice={notice}
      onCalibrate={async (id: string) => {
        'use server';
        await setCalibrationAction(id);
      }}
    />
  );
}

function PostRow({ post, now }: { post: LfgPostView; now: number }) {
  const where = [post.system, post.location].filter(Boolean).join(' · ');
  const details = [
    where || null,
    post.ship,
    `${post.crew_count}/${post.crew_slots} crew`,
    voiceLabel(post.voice),
    regionLabel(post.region),
    timeLeft(post.expires_at, now),
  ]
    .filter(Boolean)
    .join(' · ');
  const full = post.crew_count >= post.crew_slots;
  return (
    <div className="hp-grant" data-testid="lfg-post">
      <div className="hp-grant__who">
        <span>
          {activityLabel(post.activity)} with @{post.host_handle}
          {post.host_verified ? '' : ' · RSI handle not verified'}
        </span>
        <span className="hp-grant__note">{details}</span>
        {post.note ? <span className="hp-grant__note">“{post.note}”</span> : null}
      </div>
      <div className="hp-grant__act-btns">
        {post.my_status === 'accepted' ? (
          <>
            <BeamChip tone="good">You&apos;re in</BeamChip>
            <CopyHandleButton handle={post.host_handle} verified={post.host_verified} />
          </>
        ) : post.my_status === 'requested' ? (
          <BeamChip tone="warn">Asked</BeamChip>
        ) : null}
        {post.is_host ? (
          <BeamChip>Your post</BeamChip>
        ) : post.my_status === 'accepted' || post.my_status === 'requested' ? (
          <form action={leaveAction}>
            <input type="hidden" name="id" value={post.id} />
            <ConfirmSubmitButton className="hp-btn hp-btn--ghost">
              {post.my_status === 'accepted' ? 'Leave' : 'Withdraw'}
            </ConfirmSubmitButton>
          </form>
        ) : post.my_status === 'removed' ? null : (
          full ? (
            <button type="button" className="hp-btn" disabled>
              Full
            </button>
          ) : (
            <form action={joinAction}>
              <input type="hidden" name="id" value={post.id} />
              <ConfirmSubmitButton className="hp-btn">Ask to join</ConfirmSubmitButton>
            </form>
          )
        )}
      </div>
      {post.is_host ? null : (
        <details className="hp-report">
          <summary>Report this post</summary>
          <form action={reportAction} className="hp-formcol">
            <input type="hidden" name="id" value={post.id} />
            <BeamSelect id={`lfg-report-reason-${post.id}`} name="reason" label="Reason" defaultValue="abuse">
              <option value="abuse">Abusive or harassing</option>
              <option value="spam">Spam or advertising</option>
              <option value="illegal_content">Illegal content</option>
              <option value="other">Something else</option>
            </BeamSelect>
            <BeamTextarea
              id={`lfg-report-details-${post.id}`}
              name="details"
              label="Details"
              maxLength={500}
              rows={2}
              hint="Optional. Only moderators see this."
            />
            <ConfirmSubmitButton className="hp-btn hp-btn--ghost">Send report</ConfirmSubmitButton>
          </form>
        </details>
      )}
    </div>
  );
}

function HostPanel({ detail, now }: { detail: LfgPostDetail; now: number }) {
  const asking = detail.members.filter((m) => m.status === 'requested');
  const crew = detail.members.filter((m) => m.status === 'accepted');
  return (
    <>
      <p className="hp-prose">
        {activityLabel(detail.activity)} · {detail.crew_count}/{detail.crew_slots} crew ·{' '}
        {timeLeft(detail.expires_at, now)}
      </p>
      <h3 className="hp-subheading">Asking to join ({asking.length})</h3>
      {asking.length === 0 ? (
        <p className="hp-prose">Nobody yet.</p>
      ) : (
        asking.map((m) => (
          <div className="hp-grant" key={m.handle} data-testid="lfg-asking">
            <div className="hp-grant__who">
              <span>@{m.handle}</span>
            </div>
            <div className="hp-grant__act-btns">
              {(['accept', 'decline'] as const).map((action) => (
                <form action={respondAction} key={action}>
                  <input type="hidden" name="id" value={detail.id} />
                  <input type="hidden" name="handle" value={m.handle} />
                  <input type="hidden" name="action" value={action} />
                  <ConfirmSubmitButton className={action === 'accept' ? 'hp-btn' : 'hp-btn hp-btn--ghost'}>
                    {action === 'accept' ? 'Accept' : 'Decline'}
                  </ConfirmSubmitButton>
                </form>
              ))}
            </div>
          </div>
        ))
      )}
      <h3 className="hp-subheading">Crew ({crew.length})</h3>
      {crew.length === 0 ? (
        <p className="hp-prose">Nobody accepted yet.</p>
      ) : (
        crew.map((m) => (
          <div className="hp-grant" key={m.handle} data-testid="lfg-crew">
            <div className="hp-grant__who">
              <span>@{m.handle}</span>
            </div>
            <div className="hp-grant__act-btns">
              <CopyHandleButton handle={m.handle} verified />
              <form action={respondAction}>
                <input type="hidden" name="id" value={detail.id} />
                <input type="hidden" name="handle" value={m.handle} />
                <input type="hidden" name="action" value="remove" />
                <ConfirmSubmitButton
                  className="hp-btn hp-btn--ghost"
                  confirm={`Remove @${m.handle}? They will not be able to ask again.`}
                >
                  Remove
                </ConfirmSubmitButton>
              </form>
            </div>
          </div>
        ))
      )}
      <form action={closePostAction} style={{ marginTop: 12 }}>
        <input type="hidden" name="id" value={detail.id} />
        <ConfirmSubmitButton className="hp-btn hp-btn--ghost" confirm="Close your post? It leaves the board.">
          Close post
        </ConfirmSubmitButton>
      </form>
    </>
  );
}

function CrewWindow({ window: w, now }: { window: CommendWindow; now: number }) {
  return (
    <div data-testid="commend-window" style={{ marginBottom: 18 }}>
      <h3 className="hp-subheading">
        {activityLabel(w.activity)} · commend within {timeLeft(w.closes_at, now).replace(' left', '')}
      </h3>
      {w.crew.map((m) => (
        <div className="hp-grant" key={m.handle} data-testid="commend-mate">
          <div className="hp-grant__who">
            <span>@{m.handle}</span>
            <span className="hp-grant__note">
              {m.my_commend ? `You said: ${commendLabel(m.my_commend)}` : 'Not commended yet'}
            </span>
          </div>
          <div className="hp-grant__act-btns" role="group" aria-label={`Commend @${m.handle}`}>
            {COMMEND_KINDS.map((kind) => (
              <form action={commendAction} key={kind} style={{ margin: 0 }}>
                <input type="hidden" name="post_id" value={w.post_id} />
                <input type="hidden" name="handle" value={m.handle} />
                <input type="hidden" name="kind" value={kind} />
                <ConfirmSubmitButton
                  className={m.my_commend === kind ? 'hp-btn' : 'hp-btn hp-btn--ghost'}
                  aria-pressed={m.my_commend === kind}
                >
                  {commendLabel(kind)}
                </ConfirmSubmitButton>
              </form>
            ))}
            {m.my_commend ? (
              <form action={commendAction} style={{ margin: 0 }}>
                <input type="hidden" name="post_id" value={w.post_id} />
                <input type="hidden" name="handle" value={m.handle} />
                <input type="hidden" name="intent" value="withdraw" />
                <ConfirmSubmitButton className="hp-btn hp-btn--ghost">Withdraw</ConfirmSubmitButton>
              </form>
            ) : null}
          </div>
        </div>
      ))}
    </div>
  );
}

function CrewHistory({ history }: { history: CrewMate[] }) {
  return (
    <Plane tilt="flat" style={{ marginTop: 18 }}>
      {history.map((m) => (
        <div className="hp-grant" key={`${m.post_id}-${m.handle}`} data-testid="crew-history">
          <div className="hp-grant__who">
            <span>@{m.handle}</span>
            <span className="hp-grant__note">
              {activityLabel(m.activity)} ·{' '}
              {new Date(m.flew_at).toLocaleDateString('en-GB', { day: 'numeric', month: 'short' })}
            </span>
          </div>
          <div className="hp-grant__act-btns">
            <CopyHandleButton handle={m.handle} verified />
          </div>
        </div>
      ))}
    </Plane>
  );
}
