import React from 'react';
import { getSessions, getBiggestTrade, getRecords } from '@/lib/api';
import { rangeToHours } from '@/lib/range';
import { loadAllReferenceBundles, type ReferenceCatalog } from '@/lib/reference';
import { EntityLink } from '@/components/kb/EntityLink';
import { logger } from '@/lib/logger';
import { defineWidget } from './kit/defineWidget';
import { ReadoutGroup, RankedList, type Readout, type Row } from './kit/archetypes';
import { fmtDuration, fmtNum } from './kit/format';
import { InfoTip } from '@/components/hud/InfoTip';
import { INFERENCE_EXPLANATIONS } from '@/lib/inference-explanations';

/** Range-windowed records (last N hours) shown alongside the lifetime bests
 *  so a range-scoped dashboard's records match the selected window. */
interface RecordsWindow {
  hours: number;
  longestSessionSecs: number;
  longestSurvivalStreakSecs: number;
}

interface RecordsData {
  longestSessionSecs: number;
  busiestSessionEvents: number;
  // Quantity of the largest confirmed transaction. The CommerceTransactionDto
  // doesn't expose a typed aUEC field; quantity is unit-count for commodity
  // trades and item-count for shop buys. Not strictly "biggest by aUEC" but
  // the closest available signal without parsing raw payloads.
  biggestTradeQuantity: number;
  biggestTradeItem: string | null;
  // Gap (secs) between the user's two most-spaced consecutive player_death
  // events. "Longest stretch alive" — only meaningful if the user has at
  // least 2 deaths in the recent window.
  longestSurvivalStreakSecs: number;
  // Highest count of player_death events in a single session. Computed
  // server-side over the full history via GET /v1/me/stats/records (F9).
  deadliestSessionDeaths: number;
  // Death- and commerce-derived rows are me-scoped; only the owner sees them
  // (C2 — a visitor would otherwise blend the viewer's own data).
  includePersonal: boolean;
  // Range-scoped bests for the selected window (owner-only; null for
  // visitors or the 'all' range where the window == lifetime).
  window: RecordsWindow | null;
  // Items catalog for deep-linking the biggest-trade item to the KB.
  // Owner-only (loaded alongside the me-scoped trade); null for visitors.
  items: ReferenceCatalog | null;
}

export const recordsWidget = defineWidget<RecordsData>({
  id: 'records',
  eyebrow: 'Records',
  // Range-aware for the SPLIT view: lifetime bests + the selected window's
  // bests. Re-queries when the range changes to refresh the window.
  rangeAware: true,
  // Owner always sees their own widget; a visitor needs the owner's
  // per-widget `records` share toggle (Plan 3b Option A).
  visibility: { shareScope: 'records' },
  async load(ctx) {
    if (!ctx.token) return null;
    const token = ctx.token;
    // The window == lifetime for 'all', so skip the redundant windowed pass.
    const hours = ctx.range === 'all' ? undefined : rangeToHours(ctx.range);

    // Death/commerce records are me-scoped, so they're only shown on the
    // owner's own view (a visitor would otherwise blend the VIEWER's data
    // into the owner's records). Used both to gate fetches here and the
    // rows in the body below.
    const includePersonal = ctx.isOwner;

    const r: RecordsData = {
      longestSessionSecs: 0,
      busiestSessionEvents: 0,
      biggestTradeQuantity: 0,
      biggestTradeItem: null,
      longestSurvivalStreakSecs: 0,
      deadliestSessionDeaths: 0,
      includePersonal,
      window: null,
      items: null,
    };

    if (includePersonal) {
      // Owner: the server computes the session/death records AND the
      // biggest confirmed trade over the FULL history (audit F9) — no
      // client-side raw-event math and no fetch caps. Parallel so one
      // failure doesn't blank the rest.
      const [recordsRes, tradeRes] = await Promise.allSettled([
        getRecords(token, hours),
        getBiggestTrade(token),
      ]);

      if (recordsRes.status === 'fulfilled' && recordsRes.value) {
        const rec = recordsRes.value;
        r.longestSessionSecs = rec.longest_session_secs;
        r.busiestSessionEvents = rec.busiest_session_events;
        r.longestSurvivalStreakSecs = rec.longest_survival_streak_secs;
        r.deadliestSessionDeaths = rec.deadliest_session_deaths;
        r.window = rec.window
          ? {
              hours: rec.window.hours,
              longestSessionSecs: rec.window.longest_session_secs,
              longestSurvivalStreakSecs: rec.window.longest_survival_streak_secs,
            }
          : null;
      } else if (recordsRes.status === 'rejected') {
        logger.warn(
          { err: recordsRes.reason, call: 'widget.records.records' },
          'fetch failed',
        );
      }

      if (tradeRes.status === 'fulfilled' && tradeRes.value?.quantity != null) {
        r.biggestTradeQuantity = tradeRes.value.quantity;
        r.biggestTradeItem = tradeRes.value.item ?? null;
      } else if (tradeRes.status === 'rejected') {
        logger.warn(
          { err: tradeRes.reason, call: 'widget.records.biggest_trade' },
          'fetch failed',
        );
      }

      // Items catalog for the biggest-trade KB link. Only needed when a
      // trade item exists; the bundle build is memoised + degrades to an
      // empty catalog (plain text) on failure.
      if (r.biggestTradeItem) {
        try {
          const { catalogs } = await loadAllReferenceBundles();
          r.items = catalogs.items;
        } catch (err) {
          logger.warn(
            { err, call: 'widget.records.items_catalog' },
            'reference load failed',
          );
        }
      }
    } else {
      // Visitor: sessions are handle-scoped + share-gated, so longest +
      // busiest are the owner's. There's no handle-scoped records
      // endpoint, so the death-derived rows stay omitted (body gates on
      // `includePersonal`).
      try {
        const sessionsRes = await getSessions(token, ctx.ownerHandle);
        for (const s of sessionsRes?.sessions ?? []) {
          if (s.started_at && s.ended_at) {
            const a = new Date(s.started_at).getTime();
            const b = new Date(s.ended_at).getTime();
            if (!Number.isNaN(a) && !Number.isNaN(b) && b > a) {
              const secs = (b - a) / 1000;
              if (secs > r.longestSessionSecs) r.longestSessionSecs = secs;
            }
          }
          if (s.event_count > r.busiestSessionEvents) {
            r.busiestSessionEvents = s.event_count;
          }
        }
      } catch (err) {
        logger.warn({ err, call: 'widget.records.sessions' }, 'fetch failed');
      }
    }

    if (
      r.longestSessionSecs === 0 &&
      r.busiestSessionEvents === 0 &&
      r.biggestTradeQuantity === 0 &&
      r.longestSurvivalStreakSecs === 0 &&
      r.deadliestSessionDeaths === 0
    ) {
      return null;
    }

    return r;
  },
  body(data, _ctx, size) {
    // Range-scoped bests shown under the lifetime bests (the "split" view).
    const win = data.window;
    const rangeDays = win ? Math.max(1, Math.round(win.hours / 24)) : 0;
    const winNote =
      win && (win.longestSessionSecs > 0 || win.longestSurvivalStreakSecs > 0)
        ? `Last ${rangeDays}d — longest ${
            win.longestSessionSecs > 0 ? fmtDuration(win.longestSessionSecs) : '—'
          }${
            win.longestSurvivalStreakSecs > 0
              ? `, survived ${fmtDuration(win.longestSurvivalStreakSecs)}`
              : ''
          }`
        : undefined;

    if (size === 'compact') {
      // Lead with longest-session (oldest record, always meaningful) and
      // survival-streak if present (most interesting new record); fall back
      // to busiest-by-events when streak is empty.
      const readouts: Readout[] = [
        {
          label: 'longest',
          value: data.longestSessionSecs > 0 ? fmtDuration(data.longestSessionSecs) : '—',
        },
        data.longestSurvivalStreakSecs > 0
          ? { label: 'survival', value: fmtDuration(data.longestSurvivalStreakSecs) }
          : { label: 'busiest', value: `${fmtNum(data.busiestSessionEvents)} ev` },
      ];
      return <ReadoutGroup readouts={readouts} note={winNote} />;
    }

    const rows: Row[] = [
      {
        key: 'longest',
        label: 'Longest session',
        value: data.longestSessionSecs > 0 ? fmtDuration(data.longestSessionSecs) : '—',
      },
      {
        key: 'busiest',
        label: 'Busiest session',
        value:
          data.busiestSessionEvents > 0 ? `${fmtNum(data.busiestSessionEvents)} ev` : '—',
      },
      // Death- and commerce-derived rows are me-scoped; omit for visitors
      // (C2) — they'd otherwise blend the viewer's data.
      ...(data.includePersonal
        ? [
            {
              key: 'deadliest',
              label: 'Deadliest session',
              value:
                data.deadliestSessionDeaths > 0
                  ? `${fmtNum(data.deadliestSessionDeaths)} deaths`
                  : '—',
            },
            {
              key: 'survival',
              label: 'Longest survival streak',
              value:
                data.longestSurvivalStreakSecs > 0
                  ? fmtDuration(data.longestSurvivalStreakSecs)
                  : '—',
            },
            {
              key: 'trade',
              label: (
                <>
                  Biggest trade
                  <InfoTip
                    label="the biggest trade"
                    text={INFERENCE_EXPLANATIONS.biggest_trade}
                  />
                </>
              ),
              value:
                data.biggestTradeQuantity > 0 ? (
                  <>
                    {`${fmtNum(data.biggestTradeQuantity)} units`}
                    {data.biggestTradeItem && (
                      <>
                        {' ('}
                        {/* Deep-link the traded item to the KB when the
                            catalog has it; `label` is pinned so the raw
                            item text is never rewritten. */}
                        <EntityLink
                          category="item"
                          classKey={data.biggestTradeItem}
                          catalog={data.items ?? undefined}
                          label={data.biggestTradeItem}
                        />
                        {')'}
                      </>
                    )}
                  </>
                ) : (
                  '—'
                ),
            },
          ]
        : []),
    ];
    return <RankedList rows={rows} note={winNote} />;
  },
});
