/**
 * Public / friend profile view.
 *
 * Resolution order:
 *   1. Try `/v1/public/:handle/summary` — no token. If 200, render the
 *      profile as "public".
 *   2. If 404 and the visitor is logged in, retry against
 *      `/v1/u/:handle/summary` (the share-with-user path). If 200,
 *      render as "shared with you".
 *   3. Otherwise render a generic not-found message — never disclose
 *      whether the user exists.
 *
 * Scope: summary + top types + 30-day activity heatmap. The same
 * dual-mode resolution (`public` vs `shared`) drives which `/timeline`
 * endpoint we call.
 */

import Link from 'next/link';
import type { Route } from 'next';
import {
  ApiCallError,
  getFriendScope,
  getFriendSummary,
  getMyShareScopes,
  getPublicProfile,
  getPublicShareScopes,
  getPublicSummary,
  getSummary,
  getSupporterStatus,
  type ProfileResponse,
  type PublicSummaryResponse,
  type ShareScope,
  type SupporterStatusDto,
  type WidgetShareScopesApi,
} from '@/lib/api';
import { formatEventType } from '@/lib/event-types';
import { logger } from '@/lib/logger';
import { getSession } from '@/lib/session';
import { ProfileProjection } from './_projection/ProfileProjection';
import {
  PublicProjection,
  type PublicCalloutVM,
} from './_projection/PublicProjection';
import { SHARE_SCOPES, splitShareScopes } from '@/lib/share-scopes';
import { getTheme } from '@/lib/theme';
import { navSections } from '@/lib/nav';
import { setCalibrationAction } from '@/app/me/_projection/actions';
import type { Calibration } from 'holo';
import type { ViewerCtx } from '@/app/_components/widgets/types';
import { DEFAULT_SHARE_SCOPES } from '@/app/_components/widgets/types';
import { WidgetCanvas } from '@/app/_components/widgets/WidgetCanvas';
import { EditToggle } from '@/app/_components/widgets/EditToggle';
import { EditModeProvider } from '@/app/_components/widgets/useEditMode';
import { ControlStrip } from '@/components/hud/ControlStrip';
import { InstrumentStrip } from '@/components/hud/InstrumentStrip';
import { ProfileCard } from '@/components/ProfileCard';
import { SupporterChip } from '@/components/SupporterChip';
import { RangeBar } from '@/components/journey/RangeBar';
import { parseRange } from '@/lib/range';

export const metadata = { title: "Profile" };

interface PageProps {
  params: Promise<{ handle: string }>;
  searchParams?: Promise<{ range?: string }>;
}

type View =
  | { kind: 'public'; data: PublicSummaryResponse }
  | { kind: 'shared'; data: PublicSummaryResponse }
  | { kind: 'self'; data: PublicSummaryResponse }
  | { kind: 'denied' };

async function resolveProfile(handle: string): Promise<View> {
  // 0. Self path — a signed-in user viewing their own /u/<handle>
  // would otherwise hit "denied" when their public-visibility toggle
  // is off. Short-circuit to their authenticated summary so the page
  // always works as a permalink to themselves. `claimedHandle` is
  // case-preserved from RSI so compare case-insensitively.
  const session = await getSession();
  if (
    session &&
    session.claimedHandle.toLowerCase() === handle.toLowerCase()
  ) {
    try {
      const data = await getSummary(session.token);
      // Structural cast — `SummaryResponse` and `PublicSummaryResponse`
      // share `by_type` / `claimed_handle` / `total`. Keeping the View
      // shape aligned avoids a second render path.
      return { kind: 'self', data: data as PublicSummaryResponse };
    } catch (e) {
      // 401 means the cookie outlived the server-side token; fall
      // through to the public/friend path so the user still sees
      // something useful instead of a hard error.
      if (!(e instanceof ApiCallError) || e.status !== 401) {
        logger.error({ err: e }, 'self summary fetch failed');
      }
    }
  }

  // 1. Public path — no auth.
  try {
    const data = await getPublicSummary(handle);
    return { kind: 'public', data };
  } catch (e) {
    if (!(e instanceof ApiCallError) || e.status !== 404) {
      // 503 (SpiceDB down) or any unexpected error — surface as denied
      // rather than crashing the route. Log so ops can see it.
      logger.error({ err: e }, 'public summary fetch failed');
      return { kind: 'denied' };
    }
  }

  // 2. Friend path — only if the visitor is logged in. Same 404 trap
  // applies: don't leak existence.
  if (!session) {
    return { kind: 'denied' };
  }
  try {
    const data = await getFriendSummary(session.token, handle);
    return { kind: 'shared', data };
  } catch (e) {
    if (e instanceof ApiCallError && e.status === 404) {
      return { kind: 'denied' };
    }
    if (e instanceof ApiCallError && e.status === 401) {
      // Stale cookie — fall through to denied view rather than
      // bouncing through /auth/login. The user can navigate there
      // explicitly if they want to retry as themselves.
      return { kind: 'denied' };
    }
    logger.error({ err: e }, 'friend summary fetch failed');
    return { kind: 'denied' };
  }
}

/** Fetch the owner's per-widget sharing toggles.
 *
 * - Owner viewing self: GET /v1/users/me/share-scopes (authenticated).
 * - Visitor: GET /v1/public/:handle/share-scopes (no token required).
 *
 * Falls back to all-false on any error — conservative, never over-shares.
 *
 * REPORTS WHETHER IT ACTUALLY READ THEM. The all-false fallback is the right
 * default for GATING data (never over-share), and exactly the wrong thing to
 * STATE: the page now says out loud what this pilot publishes and withholds,
 * and rendering the fallback verbatim would tell every reader "publishes
 * nothing" on the strength of a network error. Because the catch is here, a
 * `Promise.allSettled` status at the call site is always `fulfilled` and can
 * never see the difference — so `ok` carries it.
 */
async function fetchShareScopes(
  handle: string,
  isOwner: boolean,
  token: string | null,
): Promise<{ scopes: WidgetShareScopesApi; ok: boolean }> {
  try {
    if (isOwner && token) {
      return { scopes: await getMyShareScopes(token), ok: true };
    }
    return { scopes: await getPublicShareScopes(handle), ok: true };
  } catch {
    // 404 = profile not public (visitor path); any other error = degrade.
    // Either way default to all-false — safer than over-sharing.
    return { scopes: { ...DEFAULT_SHARE_SCOPES }, ok: false };
  }
}

/**
 * Plan 3b Option B — fetch the visitor's per-recipient ShareScope.
 *
 * Only meaningful for authenticated visitors who aren't the owner. Owners
 * always see their own data unconditionally (returns null). Unauthed
 * visitors have no scope row to read (returns null). Any error path also
 * collapses to null — equivalent to "no clamp", letting Option A
 * (per-owner toggles in shareScopes) be the sole gate.
 */
async function fetchRecipientScopes(
  handle: string,
  isOwner: boolean,
  token: string | null,
): Promise<ShareScope | null> {
  if (isOwner || !token) return null;
  try {
    return await getFriendScope(token, handle);
  } catch {
    return null;
  }
}

export default async function PublicProfilePage(props: PageProps) {
  const { handle } = await props.params;
  const view = await resolveProfile(handle);
  const session = await getSession();

  let calibration: Calibration = 'terra';
  try {
    calibration = (await getTheme(session?.token)) as Calibration;
  } catch {
    // Preference read failed; the default stands.
  }

  const isOwner = Boolean(
    session && session.claimedHandle.toLowerCase() === handle.toLowerCase(),
  );
  const token = session?.token ?? null;

  const sp = props.searchParams ? await props.searchParams : {};
  const range = parseRange(sp.range);

  // Fetch share scopes + recipient scope clamp once at page render.
  // Both functions catch their own errors and return safe defaults,
  // so `Promise.all` would also work in practice — but the project
  // invariant (docs/ENGINEERING.md) requires `Promise.allSettled` for any
  // multi-endpoint render path so a future refactor that drops the
  // internal catches doesn't silently start blanking the page.
  const [shareScopesResult, recipientScopesResult] = await Promise.allSettled([
    fetchShareScopes(handle, isOwner, token),
    fetchRecipientScopes(handle, isOwner, token),
  ]);
  const shareScopesRead =
    shareScopesResult.status === 'fulfilled'
      ? shareScopesResult.value
      : { scopes: { ...DEFAULT_SHARE_SCOPES }, ok: false };
  const shareScopes = shareScopesRead.scopes;
  const recipientScopes =
    recipientScopesResult.status === 'fulfilled' ? recipientScopesResult.value : null;

  const viewerCtx: ViewerCtx = {
    ownerHandle: handle,
    viewerHandle: session?.claimedHandle?.toLowerCase() ?? null,
    isOwner,
    token,
    shareScopes,
    recipientScopes,
    range,
  };

  if (view.kind === 'denied') {
    // The refused view gets the SAME chrome as a visible one. It is the
    // branch a stranger is most likely to land on, and leaving it in the
    // flat shell would have made "this profile is private" look like a
    // different site rather than a different answer.
    return (
      <ProfileProjection
        handle={session?.claimedHandle}
        calibration={calibration}
        nav={navSections({
          signedIn: Boolean(session),
          staffRoles: session?.staffRoles,
        })}
        crumb={[
          ...(session
            ? [{ label: 'Directory', href: '/discover' }]
            : [{ label: 'Site', href: '/' }]),
          { label: `@${handle}` },
        ]}
        sections={[
          {
            id: 'profile',
            title: 'Public profile',
            group: 'profile',
            node: (
          // No `role="main"`: `Projection` owns the single landmark. Still a DIV
          // rather than a `<main>` element — globals.css clamps a bare `<main>` to
          // a 720px column.
          <div
            className="ss-screen-enter"
            style={{ display: 'flex', flexDirection: 'column', gap: 20 }}
          >
            <InstrumentStrip
              title={
                <h1 className="hud-tile__title" style={{ margin: 0, fontSize: 18 }}>
                  Profile not available
                </h1>
              }
              context="Public profile"
            />
            <p
              style={{
                margin: '6px 0 0',
                color: 'var(--fg-muted)',
                fontSize: 14,
              }}
            >
              This profile either doesn&apos;t exist, isn&apos;t public, or
              hasn&apos;t been shared with you.
            </p>
          </div>
            ),
          },
        ]}
        notice={null}
        onCalibrate={async (id: string) => {
          'use server';
          await setCalibrationAction(id);
        }}
      />
    );
  }

  const { data } = view;
  const topTypes = [...data.by_type]
    .sort((a, b) => b.count - a.count)
    .slice(0, 5);

  // Fetch the citizen-profile snapshot for this handle. The endpoint
  // is unauthenticated but enforces public-or-shared visibility
  // server-side, so 404 here can mean "no snapshot yet", "not public",
  // or "not shared with you" — any of which collapse to "don't render
  // the card". Other failures degrade quietly: the rest of the page
  // is still useful.
  let profile: ProfileResponse | null = null;
  try {
    profile = await getPublicProfile(handle);
  } catch (e) {
    if (!(e instanceof ApiCallError) || e.status !== 404) {
      logger.warn({ err: e }, 'public profile snapshot fetch failed');
    }
  }

  // Supporter chip:
  //   - self path: fetch the full SupporterStatusDto via /v1/me/supporter
  //     (carries grace_until + timestamps the chip ignores; same shape
  //     SupporterChip already takes).
  //   - public / shared paths: read the public projection that
  //     `PublicSummaryResponse.supporter` now carries (state + tier +
  //     name_plate only — see `PublicSupporterInfo` server-side). The
  //     two shapes overlap on the three fields SupporterChip needs, so
  //     we feed either through the same `status` prop.
  // Fail-soft to null on any error so the rest of the profile keeps
  // rendering.
  type ChipStatus = Pick<
    SupporterStatusDto,
    'state' | 'name_plate' | 'current_tier_key'
  >;
  let chipStatus: ChipStatus | null = null;
  if (view.kind === 'self' && token) {
    try {
      chipStatus = await getSupporterStatus(token);
    } catch (e) {
      logger.warn({ err: e }, 'self supporter status fetch failed');
    }
  } else if (view.kind === 'public' || view.kind === 'shared') {
    // PublicSummaryResponse.supporter is `PublicSupporterInfo | null`;
    // structurally identical to the SupporterChip status shape.
    chipStatus = data.supporter ?? null;
  }

  /*
   * What this pilot publishes, and what they withhold.
   *
   * `Profile.jsx` states both — "Economy and Flight time are private", with the
   * note that "a public profile must never imply data it is not allowed to
   * show". The product said neither, so a reader had no way to tell a quiet
   * pilot from a private one, and the owner had no way to check what a stranger
   * actually sees without opening a private window.
   *
   * READ FROM THE SHARE SCOPES, FOR EVERYONE. This was owner-only, derived from
   * `getProfileLayoutForRender`, with a comment explaining that a visitor is
   * served the DEFAULT layout so telling them "Economy is private" would invent
   * a claim about someone else's settings. That reasoning was right about the
   * LAYOUT and wrong about the page: `shareScopes` above is the pilot's own
   * per-scope switch set, fetched from `/v1/public/{handle}/share-scopes` with
   * no token at all. It was already being fetched and handed to `WidgetCanvas`
   * without ever being read. It is the pilot's actual decision, so it can be
   * stated to anyone — which is the whole point of the screen.
   *
   * `scopesKnown` is load-bearing. A failed fetch falls back to
   * `DEFAULT_SHARE_SCOPES` — all false — and rendering that would tell every
   * reader this pilot publishes nothing, on the strength of a network error.
   */
  const scopesKnown = shareScopesRead.ok;
  const { published, withheld } = splitShareScopes(shareScopes);

  /*
   * Ring segments — the pilot's real event-type mix.
   *
   * The kit draws one segment per published lens at `1 / n` each. That is fine
   * in a mock and wrong here: equal segments draw a distribution that does not
   * exist, and every other ring in this product is proportional, so a reader
   * has every reason to read this one as proportional too. `by_type` is a real
   * public distribution and carries the ring instead; the published SET is
   * stated in the pane and in a callout, where a set belongs.
   */
  const segments = topTypes
    .map((t) => ({
      name: formatEventType(t.event_type).label,
      share: data.total > 0 ? t.count / data.total : 0,
    }))
    .filter((s) => s.share > 0);

  const enlistmentYear = profile?.enlistment_date
    ? new Date(profile.enlistment_date).getUTCFullYear()
    : null;

  /*
   * Callouts. SIX AT MOST — `CalloutField` draws three a side and reports the
   * rest rather than dropping them silently.
   *
   * Every one is a figure this page actually holds. The kit also hangs
   * "Locations seen", "Sessions shared", "Quantum transits" and "Kill / death"
   * here; `PublicSummaryResponse` is `{ claimed_handle, total, by_type,
   * supporter }` and carries none of them. The page a stranger reads is the
   * worst place in the product to fill a slot with a guess.
   */
  const callouts: PublicCalloutVM[] = [
    {
      id: 'handle',
      label: 'Handle',
      value: `@${data.claimed_handle}`,
      sub: enlistmentYear ? `Citizen since ${enlistmentYear}` : undefined,
    },
    {
      id: 'events',
      label: 'Events shared',
      value: data.total.toLocaleString(),
      sub: `${data.by_type.length} distinct types`,
    },
    ...(topTypes[0]
      ? [
          {
            id: 'top',
            label: 'Top signal',
            value: formatEventType(topTypes[0].event_type).label,
            sub: `${topTypes[0].count.toLocaleString()} logged`,
          },
        ]
      : []),
    ...(topTypes[1]
      ? [
          {
            id: 'second',
            label: 'Next signal',
            value: formatEventType(topTypes[1].event_type).label,
            sub: `${topTypes[1].count.toLocaleString()} logged`,
          },
        ]
      : []),
    ...(scopesKnown
      ? [
          {
            id: 'published',
            label: 'Published',
            value: `${published.length} of ${SHARE_SCOPES.length}`,
            sub: published.length > 0 ? published.join(' · ') : 'nothing shared',
          },
        ]
      : []),
    ...(scopesKnown && withheld.length > 0
      ? [
          {
            id: 'private',
            label: 'Private',
            value: `${withheld.length} ${withheld.length === 1 ? 'scope' : 'scopes'}`,
            sub: withheld.join(' · '),
            tone: 'warn' as const,
          },
        ]
      : []),
  ];

  const subStats = [
    { k: 'Events', v: data.total.toLocaleString() },
    { k: 'Types', v: String(data.by_type.length) },
    {
      k: 'Top signal',
      v: topTypes[0]
        ? formatEventType(topTypes[0].event_type).label
        : '—',
    },
    ...(scopesKnown
      ? [
          {
            k: 'Published',
            v: `${published.length}/${SHARE_SCOPES.length}`,
            ...(published.length === 0 ? { tone: 'warn' as const } : null),
          },
        ]
      : []),
  ];

  return (
    <PublicProjection
      subject={data.claimed_handle}
      handle={session?.claimedHandle ?? null}
      kind={view.kind}
      calibration={calibration}
      nav={navSections({
        signedIn: Boolean(session),
        staffRoles: session?.staffRoles,
      })}
      crumb={[
        ...(session
          ? [{ label: 'Directory', href: '/discover' }]
          : [{ label: 'Site', href: '/' }]),
        { label: `@${handle}` },
      ]}
      total={data.total.toLocaleString()}
      totalDetail={
        scopesKnown
          ? `${data.by_type.length} event types · ${published.length} of ${SHARE_SCOPES.length} scopes published`
          : `${data.by_type.length} event types`
      }
      segments={segments}
      callouts={callouts}
      subStats={subStats}
      published={published}
      withheld={withheld}
      scopesKnown={scopesKnown}
      chips={
        <div className="hp-chiprow">
          {view.kind === 'self' ? (
            <span className="hp-chip">You</span>
          ) : view.kind === 'public' ? (
            <span className="hp-chip">Public</span>
          ) : (
            <span className="hp-chip">Shared with you</span>
          )}
          {profile ? <span className="hp-chip">RSI verified</span> : null}
          <SupporterChip status={chipStatus} />
        </div>
      }
      body={
        <div className="hp-recgroup" style={{ marginTop: 18 }}>
          {profile ? <ProfileCard profile={profile} /> : null}

          {/* The range control is the OWNER's, and only theirs: a visitor is
              reading a fixed public summary, so a window picker would imply
              the figures move with it. */}
          {isOwner ? (
            <RangeBar
              active={range}
              buildHref={(id) => `/u/${handle}?range=${id}` as Route}
            />
          ) : null}

          <EditModeProvider>
            {isOwner && (
              <ControlStrip>
                <span style={{ flex: 1 }} />
                <EditToggle />
              </ControlStrip>
            )}
            <WidgetCanvas ctx={viewerCtx} surface="profile" />
          </EditModeProvider>

          {view.kind === 'public' && (
            <p className="hp-note">
              This is the public view — summary, top types, and a coarse
              activity heatmap only. The detailed timeline is only visible to
              handles or orgs the owner has explicitly shared with.
            </p>
          )}
          {view.kind === 'shared' && (
            <p className="hp-note">
              <span className="mono">{data.claimed_handle}</span> has shared
              their manifest with you, so you see the full summary + timeline
              that public viewers don&apos;t.{' '}
              <Link
                href={
                  `/sharing?handle=${encodeURIComponent(data.claimed_handle)}` as Route
                }
              >
                Share back &rarr;
              </Link>
            </p>
          )}
        </div>
      }
      onCalibrate={async (id: string) => {
        'use server';
        await setCalibrationAction(id);
      }}
    />
  );
}
