import React from 'react';
import Link from 'next/link';
import {
  resolveContractNames,
  contractNameHref,
  normalizeContractName,
} from '@/lib/contracts';
import type { Route } from 'next';
import { getContracts } from '@/lib/api';
import { logger } from '@/lib/logger';
import { rangeToWindowHours } from '@/lib/range';
import { defineWidget } from './kit/defineWidget';
import { ReadoutGroup } from './kit/archetypes';
import { fmtNum, fmtPct } from './kit/format';
import { InfoTip } from '@/components/hud/InfoTip';
import { INFERENCE_EXPLANATIONS } from '@/lib/inference-explanations';

/**
 * `contracts` — contract-run outcomes from `GET /v1/me/stats/contracts`
 * (the materialised `contract_runs` rollup, derived from HUD notification
 * banners keyed by mission id).
 *
 * `withdrawn` and `unknown` are reported by the API and distinguished
 * deliberately by the fold, but are NOT shown here: at 6 and 77 of 609
 * runs in the reference corpus they don't earn tile space against
 * `abandoned` (189). They surface on the contract-history view instead.
 *
 * `completion_pct` = completed / (completed + failed + abandoned) —
 * in-progress runs are excluded because they haven't resolved. `null`
 * when nothing has resolved; rendered as an em dash, never 0%.
 *
 * The headline `done <completed>/<resolved>` and the `rate` meter share
 * that SAME denominator (`completed + failed + abandoned`) — NOT the
 * API's `total`, which also counts `withdrawn`/`in_progress`/`unknown`
 * runs that haven't resolved either way. A headline denominated on
 * `total` next to a rate denominated on `resolved` is exactly the bug
 * this widget and `objectives` both had: two numbers on one tile
 * silently describing different populations.
 *
 * "Most run" picks the highest count among `res.runs`, filtered to the
 * same outcome states the headline counts (see the `COUNTED` allowlist
 * below — `runs` intentionally also includes `Superseded` re-accept rows
 * that never earn a headline count). Ties resolve to whichever name's
 * run was inserted into the `Map` first, which is only deterministic
 * because the server orders `runs` by `accepted_at DESC, mission_id ASC`
 * (`repo.rs`'s `contract_runs` query) — a future change to that ORDER BY
 * would silently make the tile's tie-break flicker between renders.
 *
 * Owner-only (me-scoped endpoint, no friend equivalent) and range-aware.
 */
interface ContractsData {
  completed: number;
  /** completed + failed + abandoned — the SAME denominator `pct` uses. */
  resolved: number;
  inProgress: number;
  abandoned: number;
  failed: number;
  /** 0–100, or null when nothing has resolved. */
  pct: number | null;
  topName: string | null;
  /** Where the top name links, or null to keep it plain text. */
  topHref: string | null;
  topCount: number;
}

export const contractsWidget = defineWidget<ContractsData>({
  id: 'contracts',
  eyebrow: 'Contracts',
  rangeAware: true,
  visibility: 'owner',
  async load(ctx) {
    if (!ctx.token) return null;
    try {
      const res = await getContracts(ctx.token, rangeToWindowHours(ctx.range));
      if (!res || res.total === 0) return null;

      // Most-run contract by name, from the returned runs. `runs`
      // intentionally includes `Superseded` rows (real history for the
      // mission) that `total` and the headline buckets exclude — see the
      // `ContractsResponse` doc. An allowlist, not an exclusion of the
      // literal `'superseded'`, so a future non-outcome state the fold
      // gains still lands here excluded by default rather than silently
      // getting counted.
      const COUNTED = new Set([
        'completed',
        'failed',
        'withdrawn',
        'abandoned',
        'in_progress',
        'unknown',
      ]);
      const counts = new Map<string, number>();
      for (const r of res.runs) {
        if (!COUNTED.has(r.state)) continue;
        counts.set(r.name, (counts.get(r.name) ?? 0) + 1);
      }
      let topName: string | null = null;
      let topCount = 0;
      for (const [name, n] of counts) {
        if (n > topCount) {
          topName = name;
          topCount = n;
        }
      }

      // Resolve the one name this widget shows. Ambiguous names go to
      // the filtered list rather than an arbitrary contract.
      let topHref: string | null = null;
      if (topName) {
        const resolved = await resolveContractNames([topName]);
        topHref = contractNameHref(topName, resolved.get(normalizeContractName(topName)));
      }

      return {
        completed: res.completed,
        resolved: res.completed + res.failed + res.abandoned,
        inProgress: res.in_progress,
        abandoned: res.abandoned,
        failed: res.failed,
        pct: res.completion_pct ?? null,
        topName,
        topHref,
        topCount,
      };
    } catch (err) {
      logger.warn({ err, call: 'widget.contracts' }, 'fetch failed');
      return null;
    }
  },
  body(data) {
    return (
      <div className="hud-readout-stack">
        <ReadoutGroup
          readouts={[
            { label: 'done', value: `${fmtNum(data.completed)}/${fmtNum(data.resolved)}` },
            {
              label: 'rate',
              info: (
                <InfoTip
                  label="the completion rate"
                  text={INFERENCE_EXPLANATIONS.contract_outcomes}
                />
              ),
              value: data.pct === null ? '—' : fmtPct(data.pct, true),
            },
          ]}
        />
        <div className="hud-meter" aria-hidden="true">
          <span
            className="hud-meter__fill"
            style={{ ['--val' as string]: `${data.pct ?? 0}%` } as React.CSSProperties}
          />
        </div>
        <ReadoutGroup
          readouts={[
            { label: 'in progress', value: fmtNum(data.inProgress), secondary: true },
            { label: 'abandoned', value: fmtNum(data.abandoned), secondary: true },
            { label: 'failed', value: fmtNum(data.failed), secondary: true },
          ]}
        />
        {data.topName ? (
          <p className="hud-note" style={{ display: 'flex', gap: 4 }}>
            <span style={{ flexShrink: 0 }}>most run:</span>
            <span className="hud-trunc">
              {data.topHref ? (
                <Link href={data.topHref as Route} prefetch={false}>
                  {data.topName}
                </Link>
              ) : (
                data.topName
              )}
            </span>
            <span style={{ flexShrink: 0 }}>×{fmtNum(data.topCount)}</span>
          </p>
        ) : null}
        <p className="hud-note">
          <Link href={'/me/contracts' as Route}>View contract history →</Link>
        </p>
      </div>
    );
  },
});
