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
 */
async function fetchShareScopes(
  handle: string,
  isOwner: boolean,
  token: string | null,
): Promise<WidgetShareScopesApi> {
  try {
    if (isOwner && token) {
      return await getMyShareScopes(token);
    }
    return await getPublicShareScopes(handle);
  } catch {
    // 404 = profile not public (visitor path); any other error = degrade.
    // Either way default to all-false — safer than over-sharing.
    return { ...DEFAULT_SHARE_SCOPES };
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
  const shareScopes =
    shareScopesResult.status === 'fulfilled'
      ? shareScopesResult.value
      : { ...DEFAULT_SHARE_SCOPES };
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
    return (
      // role="main" over <main> element — global 720px column avoidance (M-W9).
      <div
        role="main"
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

  return (
    // role="main" over <main> element — global 720px column avoidance (M-W9).
    <div
      role="main"
      className="ss-screen-enter"
      style={{ display: 'flex', flexDirection: 'column', gap: 20 }}
    >
      <InstrumentStrip
        size="hero"
        title={
          <h1 className="hud-tile__title" style={{ margin: 0, fontSize: 'inherit' }}>
            <span className="mono">{data.claimed_handle}</span>
          </h1>
        }
        context={
          view.kind === 'self'
            ? 'Your profile'
            : view.kind === 'public'
              ? 'Public profile'
              : 'Shared with you'
        }
        trailing={
          /* Sharing CTAs — context-sensitive deep links into /sharing.
             Self view: jump to your own sharing management. Other
             users: pre-populate the add-handle field so the user can
             "share back" with a single confirm click. */
          view.kind === 'self' ? (
            <Link
              href="/sharing"
              className="ss-btn ss-btn--ghost"
              style={{ textDecoration: 'none' }}
            >
              Manage sharing
            </Link>
          ) : (
            <Link
              href={
                (`/sharing?handle=${encodeURIComponent(data.claimed_handle)}`) as Route
              }
              className="ss-btn ss-btn--ghost"
              style={{ textDecoration: 'none' }}
            >
              Share back
            </Link>
          )
        }
      />
      <div
        style={{
          display: 'flex',
          gap: 10,
          flexWrap: 'wrap',
        }}
      >
        {view.kind === 'self' ? (
          <span className="ss-badge ss-badge--accent">
            <span className="ss-badge-dot" />
            You
          </span>
        ) : view.kind === 'public' ? (
          <span className="ss-badge ss-badge--accent">
            <span className="ss-badge-dot" />
            Public profile
          </span>
        ) : (
          <span className="ss-badge ss-badge--accent">
            Shared with you
          </span>
        )}
        {profile && (
          <span className="ss-badge ss-badge--ok">RSI verified</span>
        )}
        <SupporterChip status={chipStatus} />
      </div>

      {/* Stat tiles. Public-safe: only the totals + top type, never
          the timeline windowed counts. */}
      <div
        data-rsprow="nowrap"
        style={{ display: 'flex', gap: 12, flexWrap: 'nowrap' }}
      >
        <PublicStatTile
          eyebrow="Total events"
          value={data.total.toLocaleString()}
        />
        <PublicStatTile
          eyebrow="Event types"
          value={String(data.by_type.length)}
        />
        <PublicStatTile
          eyebrow="Top signal"
          value={
            topTypes[0] ? formatEventType(topTypes[0].event_type).label : '—'
          }
        />
        <PublicStatTile
          eyebrow="Top count"
          value={
            topTypes[0] ? topTypes[0].count.toLocaleString() : '—'
          }
        />
      </div>

      {profile && <ProfileCard profile={profile} />}

      {isOwner && (
        <RangeBar
          active={range}
          buildHref={(id) => `/u/${handle}?range=${id}` as Route}
        />
      )}
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
        <div
          style={{
            padding: '14px 18px',
            background: 'var(--bg-elev)',
            border: '1px solid var(--border)',
            borderRadius: 'var(--r-sm)',
            color: 'var(--fg-dim)',
            fontSize: 12,
            lineHeight: 1.5,
          }}
        >
          This is the public view — summary, top types, and a coarse
          activity heatmap only. The detailed timeline is only visible
          to handles or orgs the owner has explicitly shared with.
        </div>
      )}
      {view.kind === 'shared' && (
        <div
          style={{
            padding: '14px 18px',
            background: 'var(--bg-elev)',
            border: '1px solid var(--border)',
            borderRadius: 'var(--r-sm)',
            color: 'var(--fg-dim)',
            fontSize: 12,
            lineHeight: 1.5,
          }}
        >
          <span className="mono" style={{ color: 'var(--fg)' }}>
            {data.claimed_handle}
          </span>{' '}
          has shared their manifest with you, so you see the full
          summary + timeline that public viewers don&apos;t.{' '}
          <Link
            href={
              (`/sharing?handle=${encodeURIComponent(data.claimed_handle)}`) as Route
            }
            style={{ color: 'var(--accent)' }}
          >
            Share back →
          </Link>
        </div>
      )}
    </div>
  );
}

/** Lightweight stat tile — public profile variant has no delta hint. */
function PublicStatTile({
  eyebrow,
  value,
}: {
  eyebrow: string;
  value: string;
}) {
  return (
    <div
      className="ss-card"
      style={{ flex: '1 1 200px', padding: '18px 20px', minWidth: 0 }}
    >
      <div className="ss-eyebrow">{eyebrow}</div>
      <div
        className="mono"
        style={{
          fontSize: 26,
          fontWeight: 500,
          letterSpacing: '-0.015em',
          margin: '8px 0 0',
          color: 'var(--fg)',
          overflow: 'hidden',
          textOverflow: 'ellipsis',
          whiteSpace: 'nowrap',
        }}
      >
        {value}
      </div>
    </div>
  );
}


