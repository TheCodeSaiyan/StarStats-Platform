/**
 * `/me/contracts` — the owner-only contract history page.
 *
 * The drill-down the `contracts` widget tile links to ("View contract
 * history →"): the widget deliberately hides `withdrawn`/`unknown` runs
 * (too rare to earn tile space, per its own doc) and never shows
 * per-step detail at all. This page is where both surface — every
 * materialised run in the window, newest-accepted first, each with:
 *
 *   - its outcome, in plain language, derived from `closed_by` (NOT the
 *     raw `state`) — see `_lib/outcome`'s doc for why that distinction
 *     (observed HUD banner vs. inferred from a dead stream) is the point;
 *   - duration, step progress, `connected_server`, and a `partial_history`
 *     callout when the run's start was never observed;
 *   - the full per-step objective text, requested via
 *     `getContracts(token, hours, true)` — `include_steps` defaults OFF
 *     server-side, so every OTHER caller (the widget included) gets zero
 *     steps back. This page's entire reason to exist is that text.
 *
 * Owner-only: `/v1/me/stats/contracts` is me-scoped with no friend
 * equivalent — same gate as `/me/travel` and `/me/loadout`. A visitor
 * render would surface the VIEWER's own contract history. Signed-out →
 * login redirect.
 *
 * Range-aware via the same `?range=` / `parseRange` / `<RangeBar>`
 * convention as `/me/travel`.
 *
 * Volume: the endpoint has no server-side `LIMIT` (a heavy player can
 * accrue 600+ lifetime runs). This page caps rendering at
 * {@link RENDER_CAP} newest runs and says so on the page when the cap
 * bites — a silently truncated list would read as "that's all of them".
 * True server-side pagination is a tracked follow-up, not solved here.
 * The Overview section's counts are unaffected by this cap — they're the
 * server's own window-wide aggregates, not derived from the capped list.
 */

import 'server-only';
import React from 'react';
import { redirect } from 'next/navigation';
import { RecordsIndex } from '@/components/projection/RecordsIndex';
import { getSession } from '@/lib/session';
import { getContracts, statusOf, type ContractsResponse } from '@/lib/api';
import { logger } from '@/lib/logger';
import { parseRange, rangeToHours, rangeLabel } from '@/lib/range';
import { fmtNum, fmtPct } from '@/app/_components/widgets/kit/format';
import {
  Plane,
  MeterRow,
  SubStats,
  Flatline,
  BeamAlert,
  type Calibration,
} from 'holo';
import { navSections } from '@/lib/nav';
import { getTheme } from '@/lib/theme';
import { setCalibrationAction } from '@/app/me/_projection/actions';
import {
  ContractsProjection,
  type ContractsSection,
} from './_projection/ContractsProjection';
import { RunPlane } from './_projection/RunPlane';
import {
  resolveContractNames,
  contractNameHref,
  normalizeContractName,
} from '@/lib/contracts';
import { byAcceptedDesc } from './_lib/outcome';

export const metadata = { title: 'Contract history' };

interface PageProps {
  searchParams?: Promise<{ range?: string }>;
}

/** Hard cap on rendered runs — see the module doc's "Volume" note. */
const RENDER_CAP = 200;

export default async function ContractsPage(props: PageProps) {
  const session = await getSession();
  if (!session) redirect('/auth/login?next=/me/contracts');

  const token = session.token;
  const sp = props.searchParams ? await props.searchParams : {};
  const range = parseRange(sp.range);
  const hours = rangeToHours(range);

  // The beam for this render; falls back to the system default rather than
  // failing the page.
  let calibration: Calibration = 'terra';
  try {
    calibration = (await getTheme(token)) as Calibration;
  } catch {
    // Preference read failed; the default stands.
  }

  // Single-endpoint render. `include_steps=true` is the whole point of
  // this page (see module doc) — every other caller of `getContracts`
  // omits it and gets back an empty `steps` on every run.
  let res: ContractsResponse | null = null;
  try {
    res = await getContracts(token, hours, true);
  } catch (err) {
    logger.warn(
      { err, call: 'me.contracts.history', status: statusOf(err) },
      'fetch failed',
    );
  }

  const runs = res?.runs ?? [];
  const sorted = [...runs].sort(byAcceptedDesc);
  // ONE request for every name on the page. A request per card would
  // burst the per-IP governor the way the KB prefetch storm did.
  const nameLinks = await resolveContractNames(sorted.map((r) => r.name));
  const shown = sorted.slice(0, RENDER_CAP);
  const droppedCount = sorted.length - shown.length;

  // Same denominator convention as the `contracts` widget: `done`/`rate`
  // share `completed + failed + abandoned`, NOT the API's `total` (which
  // also counts in-progress/withdrawn/unknown runs that haven't resolved
  // either way) — see the widget's doc for the bug this avoids repeating.
  const resolved = res ? res.completed + res.failed + res.abandoned : 0;

  // ---------------------------------------------------------------------
  // Sections. Two groups: the window's outcome counts, and the runs.
  // ---------------------------------------------------------------------
  const sections: ContractsSection[] = res
    ? [
        {
          id: 'overview',
          title: 'Outcomes',
          ctx: rangeLabel(range),
          group: 'outcomes',
          node: (
            <>
              {/* `Records.jsx` puts a pilot's own records behind one category
                  strip so they read as a family; the product had four unrelated
                  routes, each a dead end. */}
              <RecordsIndex active="/me/contracts" />
            <>
              {/* The denominator is `completed + failed + abandoned` — the
                  RESOLVED runs — and NOT the API's `total`, which also counts
                  in-progress, withdrawn and unknown runs that have not landed
                  either way. Using `total` would report a completion rate that
                  falls every time a contract is accepted. */}
              <SubStats
                items={[
                  {
                    k: 'Done',
                    v: `${fmtNum(res.completed)}/${fmtNum(resolved)}`,
                  },
                  {
                    k: 'Rate',
                    v:
                      res.completion_pct == null
                        ? '—'
                        : fmtPct(res.completion_pct, true),
                  },
                  { k: 'In progress', v: fmtNum(res.in_progress) },
                  { k: 'Abandoned', v: fmtNum(res.abandoned) },
                ]}
              />
              <Plane
                tilt="flat"
                cap="Every outcome"
                hint={rangeLabel(range)}
                style={{ marginTop: 20 }}
              >
                {(
                  [
                    ['Completed', res.completed],
                    ['Failed', res.failed],
                    ['Abandoned', res.abandoned],
                    ['In progress', res.in_progress],
                    ['Withdrawn', res.withdrawn],
                    ['Unknown', res.unknown],
                  ] as const
                ).map(([label, n], i) => (
                  <MeterRow
                    key={label}
                    rank={i + 1}
                    name={label}
                    value={fmtNum(n)}
                    valueText
                  />
                ))}
              </Plane>
              <p className="hp-prose">
                {/* State the limit: "abandoned" is INFERRED from a stream that
                    went dead, not an observed outcome. */}
                Abandoned and unknown runs were closed by inference — the game
                showed no banner for them, so they are read from a stream that
                stopped rather than from an outcome it reported.
              </p>
            </>
            </>
          ),
        },
        {
          id: 'runs',
          title: 'Contract history',
          ctx: `${shown.length.toLocaleString()} shown · ${rangeLabel(range)}`,
          group: 'runs',
          node:
            shown.length === 0 ? (
              <Flatline reason="no-data" />
            ) : (
              <>
                {droppedCount > 0 ? (
                  // A silently truncated list would read as "that's all of
                  // them", so the cap announces itself.
                  <p className="hp-prose">
                    Showing the {RENDER_CAP.toLocaleString()} most recent runs
                    of {sorted.length.toLocaleString()} in this window —{' '}
                    {droppedCount.toLocaleString()} older runs aren&apos;t
                    shown.
                  </p>
                ) : null}
                {shown.map((run, i) => (
                  <RunPlane
                    key={`${run.mission_id}:${i}`}
                    run={run}
                    href={contractNameHref(
                      run.name,
                      nameLinks.get(normalizeContractName(run.name)),
                    )}
                  />
                ))}
              </>
            ),
        },
      ]
    : [];

  return (
    <ContractsProjection
      handle={session.claimedHandle}
      calibration={calibration}
      range={range}
      nav={navSections(
        { signedIn: true, staffRoles: session.staffRoles },
        'contracts',
      )}
      sections={sections}
      notice={null}
      banner={
        res === null ? (
          <BeamAlert tone="bad">
            Couldn&apos;t load contract history — the contracts service
            didn&apos;t respond. Try reloading.
          </BeamAlert>
        ) : null
      }
      onCalibrate={async (id: string) => {
        'use server';
        await setCalibrationAction(id);
      }}
    />
  );
}
