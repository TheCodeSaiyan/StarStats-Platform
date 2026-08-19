/**
 * `/me` — the private home page ("mirror").
 *
 * Three stacked surfaces:
 *   1. <MeIdentityHeader> — lifetime "who am I": handle, supporter
 *      chip, enlistment year, and a lifetime stat row (playtime,
 *      events, locations, K/D). Range-independent.
 *   2. <RangeBar> — the prominent global time-range control (Plan 2).
 *      Drives `?range=` which the home widget canvas re-queries on.
 *   3. <WidgetCanvas surface="home"> — the configurable widget grid
 *      bound to the `home` layout surface (shares the render path with
 *      the public `/u/[handle]` profile, which uses `surface="profile"`).
 *
 * Fetching follows the docs/ENGINEERING.md invariant: every multi-endpoint render
 * uses `Promise.allSettled`, never `Promise.all`, so one endpoint
 * hiccup degrades a single number rather than blanking the page. Each
 * rejection is logged with its `call=` label.
 */

import type { Route } from 'next';
import { redirect } from 'next/navigation';
import {
  getCombatStats,
  getLocationsVisited,
  getMyProfile,
  getSummary,
  getSupporterStatus,
  getPlaytime,
  statusOf,
  type CombatStatsResponse,
  type LocationsStatsResponse,
  type PlaytimeStatsResponse,
  type ProfileResponse,
  type SummaryResponse,
  type SupporterStatusDto,
} from '@/lib/api';
import { logger } from '@/lib/logger';
import { getSession } from '@/lib/session';
import { parseRange } from '@/lib/range';
import { RangeBar } from '@/components/journey/RangeBar';
import { MeIdentityHeader } from '@/app/_components/MeIdentityHeader';
import { WidgetCanvas } from '@/app/_components/widgets/WidgetCanvas';
import { ControlStrip } from '@/components/hud/ControlStrip';
import { EditToggle } from '@/app/_components/widgets/EditToggle';
import { EditModeProvider } from '@/app/_components/widgets/useEditMode';
import type { ViewerCtx } from '@/app/_components/widgets/types';

export const metadata = { title: "Home" };

interface PageProps {
  searchParams?: Promise<{ range?: string }>;
}

/** All-true owner share scopes — the owner sees every widget of their
 *  own data unconditionally (mirrors the `/u/[handle]` owner ctx). */
const OWNER_SHARE_SCOPES = {
  combat_mission: true,
  economy: true,
  travel: true,
  records: true,
  recent_activity: true,
} as const;

export default async function MePage(props: PageProps) {
  const session = await getSession();
  if (!session) redirect('/auth/login?next=/me');

  const token = session.token;
  const sp = props.searchParams ? await props.searchParams : {};
  const range = parseRange(sp.range);
  // The identity header is range-INDEPENDENT (spec intent): a stable
  // "who am I" anchor that doesn't shift with the RangeBar (which drives
  // only the widget canvas below, via `range`). Playtime is the true
  // all-time total via the `all_time` aggregate (uncapped). Locations and
  // combat still use the server's max window below — those endpoints are
  // windowed-only, so their header figures stay best-available (~1 year).
  const HEADER_HOURS = 24 * 365;

  // docs/ENGINEERING.md: multi-endpoint render -> Promise.allSettled. Each source
  // degrades independently; a single rejection logs its label and the
  // header falls back to a safe zero/null for that field.
  const [
    profileResult,
    supporterResult,
    summaryResult,
    playtimeResult,
    locationsResult,
    combatResult,
  ] = await Promise.allSettled([
    getMyProfile(token),
    getSupporterStatus(token),
    getSummary(token),
    getPlaytime(token, undefined, true),
    getLocationsVisited(token, HEADER_HOURS),
    getCombatStats(token, HEADER_HOURS),
  ]);

  const profile: ProfileResponse | null =
    profileResult.status === 'fulfilled' ? profileResult.value : null;
  if (profileResult.status === 'rejected') {
    logger.warn(
      {
        err: profileResult.reason,
        call: 'me.profile',
        status: statusOf(profileResult.reason),
      },
      'fetch failed',
    );
  }

  const supporter: SupporterStatusDto | null =
    supporterResult.status === 'fulfilled' ? supporterResult.value : null;
  if (supporterResult.status === 'rejected') {
    logger.warn(
      {
        err: supporterResult.reason,
        call: 'me.supporter',
        status: statusOf(supporterResult.reason),
      },
      'fetch failed',
    );
  }

  const summary: SummaryResponse | null =
    summaryResult.status === 'fulfilled' ? summaryResult.value : null;
  if (summaryResult.status === 'rejected') {
    logger.warn(
      {
        err: summaryResult.reason,
        call: 'me.summary',
        status: statusOf(summaryResult.reason),
      },
      'fetch failed',
    );
  }

  const playtime: PlaytimeStatsResponse | null =
    playtimeResult.status === 'fulfilled' ? playtimeResult.value : null;
  if (playtimeResult.status === 'rejected') {
    logger.warn(
      {
        err: playtimeResult.reason,
        call: 'me.playtime',
        status: statusOf(playtimeResult.reason),
      },
      'fetch failed',
    );
  }

  const locations: LocationsStatsResponse | null =
    locationsResult.status === 'fulfilled' ? locationsResult.value : null;
  if (locationsResult.status === 'rejected') {
    logger.warn(
      {
        err: locationsResult.reason,
        call: 'me.locations',
        status: statusOf(locationsResult.reason),
      },
      'fetch failed',
    );
  }

  const combat: CombatStatsResponse | null =
    combatResult.status === 'fulfilled' ? combatResult.value : null;
  if (combatResult.status === 'rejected') {
    logger.warn(
      {
        err: combatResult.reason,
        call: 'me.combat',
        status: statusOf(combatResult.reason),
      },
      'fetch failed',
    );
  }

  const kills = combat?.kills ?? 0;
  const deaths = combat?.deaths ?? 0;
  const deathsInferred = combat?.deaths_inferred ?? 0;

  // Supporter chip only shows for active/lapsed states; map the tier
  // key through to the header (which renders the chip when non-null).
  const supporterTier =
    supporter && (supporter.state === 'active' || supporter.state === 'lapsed')
      ? supporter.current_tier_key ?? 'standard'
      : null;

  const viewerCtx: ViewerCtx = {
    ownerHandle: session.claimedHandle,
    viewerHandle: session.claimedHandle.toLowerCase(),
    isOwner: true,
    token,
    shareScopes: { ...OWNER_SHARE_SCOPES },
    recipientScopes: null,
    range,
  };

  return (
    // `role="main"` (not a <main> element) so the global `main {}`
    // 720px legacy column in globals.css doesn't clamp this full-width
    // dashboard — same landmark for AT, zero CSS collision (M-W9).
    <div
      role="main"
      className="ss-screen-enter"
      style={{ display: 'flex', flexDirection: 'column', gap: 20 }}
    >
      <MeIdentityHeader
        handle={session.claimedHandle}
        supporterTier={supporterTier}
        enlistmentDate={profile?.enlistment_date ?? null}
        totalEvents={summary?.total ?? 0}
        deaths={deaths}
        deathsInferred={deathsInferred}
        kills={kills}
        playtimeSecs={playtime?.total_playtime_secs ?? 0}
        locationsVisited={locations?.unique_locations ?? 0}
      />

      <EditModeProvider>
        <ControlStrip>
          <RangeBar
            active={range}
            buildHref={(id) => `/me?range=${id}` as Route}
          />
          <span style={{ flex: 1 }} />
          <EditToggle />
        </ControlStrip>

        <WidgetCanvas ctx={viewerCtx} surface="home" />
      </EditModeProvider>
    </div>
  );
}
