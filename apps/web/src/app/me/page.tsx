/**
 * `/me` — the reader's own projection.
 *
 * This page WAS three stacked flat surfaces: an identity header, a RangeBar,
 * and a 24-column drag/resize widget grid. It is now one holographic volume.
 * The decisions behind the port — and the four things still degraded pending
 * backend work — are recorded in `docs/PLAN-PROJECTION-PORT.md`.
 *
 * DATA FETCHING IS UNCHANGED. Same endpoints, same `Promise.allSettled`
 * invariant, same per-call `call=` logging, same widget loaders (reached via
 * `WidgetDef.load`, which the projection reuses so every endpoint call, empty
 * check and provenance caveat stays in exactly one place). Only presentation
 * moved.
 *
 * The public profile `/u/[handle]` deliberately did NOT come along: it keeps
 * the flat `WidgetCanvas`. The two surfaces used to share one render path, but
 * a public page and a reader's own instrument want different things — no layout
 * editor, no calibration control, and unpublished lenses shown as explicitly
 * private rather than hidden.
 */

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
  getTimeline,
} from '@/lib/api';
import { logger } from '@/lib/logger';
import { getSession } from '@/lib/session';
import { parseRange } from '@/lib/range';
import { getTheme } from '@/lib/theme';
import { navSections } from '@/lib/nav';
import { getProfileLayoutForRender } from '@/lib/profile-layout';
import type { ViewerCtx } from '@/app/_components/widgets/types';
import { fmtNum } from '@/app/_components/widgets/kit/format';
import { formatPlaytime, formatKd, enlistmentYear } from '@/app/_components/MeIdentityHeader';
import { MeProjection } from './_projection/MeProjection';

/**
 * Days of daily buckets behind the projection's trace and the ring's bars.
 *
 * 26 weeks, matching the captions the design system uses for its own traces
 * ("· 26 weeks"). Range-INDEPENDENT on purpose: the trace is the shape of the
 * reader's activity over a long window, and re-cutting it to the range control
 * would make it repeat what the panes below already say.
 */
const TRACE_DAYS = 182;
import { buildElements } from './_projection/elements';
import { buildRingMap } from './_projection/ring-map';
import {
  saveProjectionLayoutAction,
  setCalibrationAction,
} from './_projection/actions';
import type { Calibration } from 'holo';

export const metadata = { title: 'Home' };

interface PageProps {
  searchParams?: Promise<{ range?: string }>;
}

/** All-true owner share scopes — the owner sees every element of their own
 *  data unconditionally (mirrors the `/u/[handle]` owner ctx). */
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

  // The identity figures are range-INDEPENDENT (spec intent): a stable "who am
  // I" anchor that doesn't shift with the range control. In the projection they
  // live in the chrome and the core readout, which is what keeps that intent
  // visible — everything inside the volume follows the range. Playtime is the
  // true all-time total via the `all_time` aggregate (uncapped); locations and
  // combat use the server's max window, so those stay best-available (~1 year).
  const HEADER_HOURS = 24 * 365;

  // docs/ENGINEERING.md: multi-endpoint render -> Promise.allSettled. Each
  // source degrades independently; a single rejection logs its label and the
  // figure falls back to a safe zero/null.
  const [
    profileResult,
    supporterResult,
    summaryResult,
    playtimeResult,
    locationsResult,
    combatResult,
    themeResult,
    timelineResult,
  ] = await Promise.allSettled([
    getMyProfile(token),
    getSupporterStatus(token),
    getSummary(token),
    getPlaytime(token, undefined, true),
    getLocationsVisited(token, HEADER_HOURS),
    getCombatStats(token, HEADER_HOURS),
    getTheme(token),
    // Real daily event counts, for the projection's trace and the ring's bars.
    // `Holotable.jsx` — the screen the whole system is designed around — puts a
    // `Trace` in the detail pane and switches the ring to `bars` for a lens;
    // both shipped missing. The kit fakes their series because it is a mock,
    // and a faked one here would be a chart of nothing presented as the
    // reader's own history.
    getTimeline(token, { days: TRACE_DAYS }),
  ]);

  const settledOr = <T,>(
    r: PromiseSettledResult<T>,
    call: string,
  ): T | null => {
    if (r.status === 'fulfilled') return r.value;
    logger.warn({ err: r.reason, call, status: statusOf(r.reason) }, 'fetch failed');
    return null;
  };

  const profile: ProfileResponse | null = settledOr(profileResult, 'me.profile');
  const supporter: SupporterStatusDto | null = settledOr(
    supporterResult,
    'me.supporter',
  );
  const summary: SummaryResponse | null = settledOr(summaryResult, 'me.summary');
  const playtime: PlaytimeStatsResponse | null = settledOr(
    playtimeResult,
    'me.playtime',
  );
  const locations: LocationsStatsResponse | null = settledOr(
    locationsResult,
    'me.locations',
  );
  const combat: CombatStatsResponse | null = settledOr(combatResult, 'me.combat');
  // The calibration falls back to the system default rather than failing the
  // page — a beam colour is not worth a 500.
  const calibration = (settledOr(themeResult, 'me.theme') ??
    'terra') as Calibration;

  const kills = combat?.kills ?? 0;
  const deaths = combat?.deaths ?? 0;
  const deathsInferred = combat?.deaths_inferred ?? 0;

  // Supporter chip only shows for active/lapsed states.
  const supporterTier =
    supporter && (supporter.state === 'active' || supporter.state === 'lapsed')
      ? (supporter.current_tier_key ?? 'standard')
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

  // The reader's saved layout, from the account. Element ids ARE widget ids, so
  // an owner who customised the flat dashboard keeps their choices.
  const timeline = settledOr(timelineResult, 'me.timeline');
  // Real summed counts per calendar day. No values, no trace — `Trace` refuses
  // to invent a series and this refuses to hand it one.
  const traceValues = (timeline?.buckets ?? []).map((b: { count: number }) => b.count);

  const layout = await getProfileLayoutForRender(
    token,
    session.claimedHandle,
    true,
    'home',
  );
  const enabledIds = layout.filter((e) => e.enabled).map((e) => e.id);

  // Element data and the ring map, both degrading independently of each other
  // and of the identity block above.
  const [elementsResult, ringMapResult] = await Promise.allSettled([
    buildElements(viewerCtx, enabledIds),
    buildRingMap(token, range),
  ]);
  const elements = settledOr(elementsResult, 'me.elements') ?? {
    callouts: [],
    planes: [],
  };
  const ringMap = settledOr(ringMapResult, 'me.ringmap') ?? {
    nodes: [],
    links: [],
    ticks: [],
  };

  return (
    <MeProjection
      handle={session.claimedHandle}
      supporterTier={supporterTier}
      enlistmentYear={enlistmentYear(profile?.enlistment_date ?? null)}
      lifetime={{
        playtime: formatPlaytime(playtime?.total_playtime_secs ?? 0),
        events: fmtNum(summary?.total ?? 0),
        locations: fmtNum(locations?.unique_locations ?? 0),
        kd: formatKd(kills, deaths),
        // K/D is derived from deaths, and deaths are partly reconstructed — so
        // a partly-guessed death count makes the RATIO partly a guess, which
        // the reader should be able to see. Stated, never rounded away.
        kdNote:
          deathsInferred > 0
            ? `Derived from deaths — ${fmtNum(deathsInferred)} of ${fmtNum(deaths)} were reconstructed from Corpse lines, as the game no longer logs deaths directly.`
            : undefined,
      }}
      calibration={calibration}
      range={range}
      enabledIds={enabledIds}
      callouts={elements.callouts}
      planes={elements.planes}
      ringMap={ringMap}
      traceValues={traceValues}
      traceDays={TRACE_DAYS}
      nav={navSections({ signedIn: true, staffRoles: session.staffRoles }, 'me')}
      onSaveLayout={async (ids: string[]) => {
        'use server';
        // The result was being dropped here, so `{ok:false}` reached the
        // editor as a resolved promise and it reported "saved to your
        // account" for a write the server had refused.
        const res = await saveProjectionLayoutAction(ids);
        return { ok: res.ok };
      }}
      onCalibrate={async (id: string) => {
        'use server';
        await setCalibrationAction(id);
      }}
    />
  );
}
