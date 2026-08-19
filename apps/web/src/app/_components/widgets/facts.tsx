import React from 'react';
import { getPlayerFacts, type PlayerFact } from '@/lib/api';
import { logger } from '@/lib/logger';
import { NoSignal } from '@/components/hud/NoSignal';
import { defineWidget } from './kit/defineWidget';
import { fmtNum } from './kit/format';

/**
 * `facts` — Player Facts (#368): fun, defensible observations about your own
 * telemetry.
 *
 * Distinct from `records` on purpose. `records` ships **superlatives**
 * (longest session, biggest trade); facts ship **observations** — patterns,
 * ratios, distributions. Without that line the two widgets converge.
 *
 *   - compact  → the single strongest fact.
 *   - expanded → up to three, each with the arithmetic behind it.
 *
 * NOT range-aware, deliberately. Every fact carries its own scope from the
 * server (`lifetime` or a trailing window); re-scoping a lifetime
 * observation to the dashboard's 24h range would make it quietly wrong —
 * the same defect class as the commerce and corridor range bugs.
 *
 * Owner-only: `/v1/me/facts` is me-scoped with no friend equivalent.
 */
interface FactsData {
  facts: readonly PlayerFact[];
  /** False when the player is too new for any claim to mean anything. */
  enoughHistory: boolean;
  sessionsConsidered: number;
  sessionsRequired: number;
}

export const factsWidget = defineWidget<FactsData>({
  id: 'facts',
  eyebrow: 'Facts',
  visibility: 'owner',
  rangeAware: false,
  async load(ctx) {
    // Owner-only defense: render() calls load() without consulting
    // isAvailable, so guard here too — never fetch me-scoped data for a
    // visitor (mirrors travel/corridors).
    if (!ctx.isOwner || !ctx.token) return null;
    try {
      const res = await getPlayerFacts(ctx.token);
      return {
        facts: res.facts ?? [],
        enoughHistory: res.enough_history,
        sessionsConsidered: res.sessions_considered,
        sessionsRequired: res.sessions_required,
      };
    } catch (err) {
      logger.warn({ err, call: 'widget.facts' }, 'fetch failed');
      return null;
    }
  },
  body(data, _ctx, size) {
    // "Too new" is a different answer from "no activity", and the canvas's
    // generic empty copy would claim the latter. Say which, and how far off
    // they are — an honest empty state, never a blank box.
    if (!data.enoughHistory) {
      return (
        <NoSignal
          compact
          title="Not enough flight time yet"
          hint={`Facts need ${fmtNum(data.sessionsRequired)} sessions to say anything meaningful — you have ${fmtNum(data.sessionsConsidered)}.`}
        />
      );
    }
    if (data.facts.length === 0) {
      // Enough history, but nothing cleared its own sample gate. Also a
      // real answer rather than an absence.
      return (
        <NoSignal
          compact
          title="Nothing stands out yet"
          hint="Your flying is steady enough that no pattern is worth calling out. Check back as the picture fills in."
        />
      );
    }

    const shown = size === 'compact' ? data.facts.slice(0, 1) : data.facts;
    return (
      <ul className="facts-list">
        {shown.map((f) => (
          <li key={f.id} className="fact">
            <span className="hud-readout fact-headline">{f.headline}</span>
            {size === 'expanded' && <p className="hud-note">{f.detail}</p>}
          </li>
        ))}
        {size === 'compact' && data.facts.length > 1 && (
          <li>
            <p className="hud-note">
              {`${fmtNum(data.facts.length - 1)} more in the expanded view`}
            </p>
          </li>
        )}
      </ul>
    );
  },
});
