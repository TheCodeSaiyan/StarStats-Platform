import React from 'react';
import { Provenance } from '@/components/Provenance';
import { getLives } from '@/lib/api';
import { rangeToHours } from '@/lib/range';
import { logger } from '@/lib/logger';
import { defineWidget } from './kit/defineWidget';
import { ReadoutGroup, type Readout } from './kit/archetypes';
import { fmtDuration, fmtNum } from './kit/format';

/** Range-windowed life stats (last N hours) shown under the lifetime ones. */
interface LivesWindow {
  hours: number;
  deaths: number;
  mean_life_secs: number | null;
}

/**
 * `lives` widget — headline life-survival stats from the character-life
 * FSM aggregate (`GET /v1/me/stats/lives`). Was the standalone
 * `<LivesTile>` on `/me`; folded into the widget registry so it's
 * movable/hideable like every other widget. Owner-only (me-scoped
 * endpoint, no friend equivalent) and lifetime — not range-aware.
 *
 * Returns body-only content: the `<Tile>` shell, eyebrow ("Character")
 * and title ("Lives") are applied by the canvas. Empty data returns null →
 * the canvas renders its shared "No data yet." placeholder.
 */
interface LivesData {
  total_lives: number;
  deaths: number;
  /** How many of `deaths` were inferred, not observed. */
  deaths_inferred: number;
  longest_life_secs: number | null;
  mean_life_secs: number | null;
  deaths_per_session: number | null;
  // Range-scoped stats for the selected window; null for 'all'.
  window: LivesWindow | null;
}

export const livesWidget = defineWidget<LivesData>({
  id: 'lives',
  eyebrow: 'Character',
  // Range-aware for the SPLIT view: lifetime stats + the selected window's.
  rangeAware: true,
  // Owner-only: /v1/me/stats/lives has no friend-scoped equivalent.
  visibility: 'owner',
  async load(ctx) {
    if (!ctx.token) return null;
    // Window == lifetime for 'all', so skip the redundant windowed pass.
    const hours = ctx.range === 'all' ? undefined : rangeToHours(ctx.range);
    let lives = null;
    try {
      lives = await getLives(ctx.token, hours);
    } catch (err) {
      logger.warn({ err, call: 'widget.lives' }, 'fetch failed');
      return null;
    }
    if (!lives || lives.total_lives === 0) return null;
    return {
      total_lives: lives.total_lives,
      deaths: lives.deaths,
      deaths_inferred: lives.deaths_inferred ?? 0,
      longest_life_secs: lives.longest_life_secs ?? null,
      mean_life_secs: lives.mean_life_secs ?? null,
      deaths_per_session: lives.deaths_per_session ?? null,
      window: lives.window
        ? {
            hours: lives.window.hours,
            deaths: lives.window.deaths,
            mean_life_secs: lives.window.mean_life_secs ?? null,
          }
        : null,
    };
  },
  body(data) {
    const streak =
      data.longest_life_secs != null ? fmtDuration(data.longest_life_secs) : '—';
    const meanLife =
      data.mean_life_secs != null ? fmtDuration(data.mean_life_secs) : '—';
    const deathsPerSession =
      data.deaths_per_session != null ? data.deaths_per_session.toFixed(1) : '—';

    const readouts: Readout[] = [
      { label: 'streak', value: streak },
      {
        label: 'deaths',
        // Marked ONLY when some were reconstructed. CIG removed the
        // Actor Death log lines, so a death is frequently derived from a
        // Corpse line rather than read — and summing them away is
        // exactly what hides that.
        value: (
          <Provenance
            total={data.deaths}
            inferred={data.deaths_inferred}
            note="reconstructed from Corpse lines, as the game no longer logs deaths directly"
          >
            {fmtNum(data.deaths)}
          </Provenance>
        ),
      },
      { label: 'deaths/session', value: deathsPerSession },
      { label: 'mean life', value: meanLife },
      { label: 'lives', value: fmtNum(data.total_lives) },
    ];

    const win = data.window;
    const rangeDays = win ? Math.max(1, Math.round(win.hours / 24)) : 0;
    const winNote =
      win && (win.deaths > 0 || win.mean_life_secs != null)
        ? `Last ${rangeDays}d — ${fmtNum(win.deaths)} deaths${
            win.mean_life_secs != null
              ? `, mean life ${fmtDuration(win.mean_life_secs)}`
              : ''
          }`
        : undefined;

    return <ReadoutGroup readouts={readouts} note={winNote} />;
  },
});
