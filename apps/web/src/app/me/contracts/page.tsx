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
import Link from 'next/link';
import type { Route } from 'next';
import { redirect } from 'next/navigation';
import { getSession } from '@/lib/session';
import { getContracts, statusOf, type ContractsResponse } from '@/lib/api';
import { logger } from '@/lib/logger';
import { parseRange, rangeToHours, rangeLabel } from '@/lib/range';
import { RangeBar } from '@/components/journey/RangeBar';
import { NoSignal } from '@/components/hud/NoSignal';
import { ReadoutGroup, type Readout } from '@/app/_components/widgets/kit/archetypes';
import { fmtNum, fmtPct } from '@/app/_components/widgets/kit/format';
import { RunCard } from './_components/RunCard';
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
  const summaryReadouts: Readout[] = res
    ? [
        { label: 'done', value: `${fmtNum(res.completed)}/${fmtNum(resolved)}` },
        {
          label: 'rate',
          value: res.completion_pct == null ? '—' : fmtPct(res.completion_pct, true),
        },
        { label: 'in progress', value: fmtNum(res.in_progress), secondary: true },
        { label: 'abandoned', value: fmtNum(res.abandoned), secondary: true },
        { label: 'failed', value: fmtNum(res.failed), secondary: true },
        { label: 'withdrawn', value: fmtNum(res.withdrawn), secondary: true },
        { label: 'unknown', value: fmtNum(res.unknown), secondary: true },
      ]
    : [];

  return (
    // `role="main"` on a DIV, not a <main> element — see /me/travel's
    // identical comment: keeps the global `main {}` 720px column from
    // clamping this full-width detail page.
    <div
      role="main"
      className="ss-screen-enter"
      style={{ display: 'flex', flexDirection: 'column', gap: 12 }}
    >
      <header style={{ display: 'flex', alignItems: 'flex-end', gap: 12, flexWrap: 'wrap' }}>
        <div>
          <div className="ss-eyebrow">Contracts</div>
          <h1
            style={{ margin: '3px 0 0', fontSize: 24, fontWeight: 600, letterSpacing: '-0.02em' }}
          >
            Contract history
          </h1>
        </div>
        <div className="hud-controls" style={{ marginLeft: 'auto', marginBottom: 0 }}>
          <Link href={'/me' as Route} className="hud-chip">
            ← Dashboard
          </Link>
          <RangeBar active={range} buildHref={(id) => `/me/contracts?range=${id}` as Route} />
        </div>
      </header>

      {res === null ? (
        <section className="hud-tile">
          <div className="hud-tile__body">
            <NoSignal
              title="Couldn't load contract history"
              hint="The contracts service didn't respond — try reloading."
            />
          </div>
        </section>
      ) : shown.length === 0 ? (
        <section className="hud-tile">
          <div className="hud-tile__body">
            <NoSignal reason="no-data" />
          </div>
        </section>
      ) : (
        <>
          <section className="hud-tile">
            <div className="hud-tile__hd">
              <span className="hud-tile__eyebrow">Outcomes</span>
              <span className="hud-tile__title">Overview</span>
              <span className="hud-tile__sub">{rangeLabel(range)}</span>
            </div>
            <div className="hud-tile__body" style={{ marginTop: 4 }}>
              <ReadoutGroup readouts={summaryReadouts} />
            </div>
          </section>

          <section className="hud-tile">
            <div className="hud-tile__hd">
              <span className="hud-tile__eyebrow">Runs</span>
              <span className="hud-tile__title">Contract history</span>
              <span className="hud-tile__sub">
                {shown.length.toLocaleString()} shown · {rangeLabel(range)}
              </span>
            </div>
            <div
              className="hud-tile__body"
              style={{ marginTop: 4, display: 'flex', flexDirection: 'column', gap: 10 }}
            >
              {droppedCount > 0 && (
                <p className="hud-note">
                  Showing the {RENDER_CAP.toLocaleString()} most recent runs of{' '}
                  {sorted.length.toLocaleString()} in this window — {droppedCount.toLocaleString()}{' '}
                  older runs aren&apos;t shown.
                </p>
              )}
              {shown.map((run, i) => (
                <RunCard
                  key={`${run.mission_id}:${i}`}
                  run={run}
                  href={contractNameHref(run.name, nameLinks.get(normalizeContractName(run.name)))}
                />
              ))}
            </div>
          </section>
        </>
      )}
    </div>
  );
}
