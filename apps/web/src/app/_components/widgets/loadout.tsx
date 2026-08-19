import React from 'react';
import Link from 'next/link';
import type { Route } from 'next';
import {
  listEvents,
  resolveReferenceItems,
  getLoadoutActivity,
} from '@/lib/api';
import { logger } from '@/lib/logger';
import {
  isExcludedPort,
  prettify,
  pickFullestLoadoutBurst,
} from '@/lib/loadout';
import type { LoadoutItem } from '@/lib/loadout';
import { EntityLink } from '@/components/kb/EntityLink';
import type { ReferenceCategory } from '@/lib/reference-types';
import { defineWidget } from './kit/defineWidget';

/**
 * `loadout` — a snapshot of current gear. Migrated to the kit:
 * `defineWidget` owns the fetch/empty/gate boilerplate; the populated
 * body stays the bespoke markup (NOT a kit archetype).
 *
 * Owner-only (`visibility: 'owner'`) — it reads owner event data.
 * Deliberately NOT range-aware: a loadout is a snapshot, not a time
 * series (a "last 7 days" filter would hide a fuller loadout captured
 * earlier). When there's no loadout snapshot, `load` returns null so the
 * tile auto-collapses to the shared compact placeholder — consistent
 * with the other widgets.
 */

// Payload shape emitted by the loadout-restore BurstSummary.
// `items` is the richer per-item list added in Task 2 (tray-side).
interface LoadoutPayload {
  kind?: string;
  categories?: Record<string, number>;
  items?: LoadoutItem[];
  last_at?: string;
}

/** One preview item — resolved to a friendly name and (when the
 *  classifier produced a slug) a KB deep-link. */
interface LoadoutPreview {
  /** Raw engine class id — the `<EntityLink>` classKey. */
  class: string;
  /** Friendly display name (resolved, or prettified fallback). */
  label: string;
  /** KB category the item links to. */
  category: ReferenceCategory;
  /** Resolver slug driving `/kb/{category}/{slug}`; null → plain text. */
  slug: string | null;
}

interface LoadoutViewData {
  count: number;
  activity: Awaited<ReturnType<typeof getLoadoutActivity>> | null;
  preview: LoadoutPreview[];
  hasMoreClasses: boolean;
}

export const loadoutWidget = defineWidget<LoadoutViewData>({
  id: 'loadout',
  eyebrow: 'Loadout',
  // A loadout is a snapshot of current gear, not a time series — it is
  // deliberately NOT range-aware (a "last 7 days" filter would hide a
  // fuller loadout captured earlier).
  rangeAware: false,
  // Loadout is owner-only — it reads owner event data.
  visibility: 'owner',
  async load(ctx) {
    if (!ctx.token) return null;
    const token = ctx.token;
    // The fullest-burst snapshot (current gear) + the equip/store activity
    // counts over time. burst_summary is denser → reaches further back.
    const [burstRes, activityRes] = await Promise.allSettled([
      listEvents(token, { event_type: 'burst_summary', limit: 200 }),
      getLoadoutActivity(token),
    ]);
    if (burstRes.status === 'rejected') {
      logger.warn({ err: burstRes.reason, call: 'widget.loadout' }, 'fetch failed');
      return null;
    }
    if (activityRes.status === 'rejected') {
      logger.warn(
        { err: activityRes.reason, call: 'widget.loadout.activity' },
        'fetch failed',
      );
    }
    const resp = burstRes.value;
    const activity = activityRes.status === 'fulfilled' ? activityRes.value : null;
    // ListEventsResponse.events is the correct field name (not .items or .data).
    if (!resp || !resp.events) return null;

    const burst = pickFullestLoadoutBurst(resp.events);
    const latest = (burst?.payload as LoadoutPayload | undefined) ?? null;

    // No loadout snapshot yet → render nothing so the tile auto-collapses
    // to the shared compact placeholder (was a bespoke hud-note before).
    if (!latest || !latest.items || latest.items.length === 0) {
      return null;
    }

    // Filter excluded ports (anatomy cosmetics + HUD mounts).
    const visibleItems = latest.items.filter((it) => !isExcludedPort(it.port));
    const count = visibleItems.length;

    // Resolve up to 3 distinct class names for friendly preview labels.
    const distinctClasses = [...new Set(visibleItems.map((it) => it.class))];
    const firstFewClasses = distinctClasses.slice(0, 3);
    // Per-class fallback category from the burst payload (weapon vs item),
    // used when the resolver has no richer classification.
    const rawCategoryByClass = new Map(
      visibleItems.map((it) => [it.class, it.category]),
    );
    let resolved: Record<
      string,
      { display_name?: string; slug?: string | null; category?: string | null }
    > = {};
    try {
      if (ctx.token && firstFewClasses.length > 0) {
        resolved = await resolveReferenceItems(ctx.token, firstFewClasses);
      }
    } catch {
      resolved = {};
    }

    const preview: LoadoutPreview[] = firstFewClasses.map((cls) => {
      const r = resolved[cls];
      // Prefer the resolver's category, then the raw burst category —
      // weapons link to the weapon KB, everything else to items.
      const rawCat = (r?.category ?? rawCategoryByClass.get(cls) ?? '').toLowerCase();
      const category: ReferenceCategory = rawCat.includes('weapon')
        ? 'weapon'
        : 'item';
      return {
        class: cls,
        label: r?.display_name ?? prettify(cls),
        category,
        slug: r?.slug ?? null,
      };
    });

    return {
      count,
      activity,
      preview,
      hasMoreClasses: distinctClasses.length > preview.length,
    };
  },
  body(data) {
    return (
      <div>
        <div className="hud-readout-wrap">
          <span className="hud-readout">
            <span className="k">items</span>
            {data.count.toLocaleString()}
          </span>
          {data.activity && (data.activity.equips > 0 || data.activity.stores > 0) && (
            <>
              <span className="hud-readout">
                <span className="k">equips</span>
                {data.activity.equips.toLocaleString()}
              </span>
              <span className="hud-readout">
                <span className="k">stores</span>
                {data.activity.stores.toLocaleString()}
              </span>
            </>
          )}
        </div>
        {data.preview.length > 0 && (
          <p className="hud-note">
            {data.preview.map((p, i) => (
              <React.Fragment key={p.class}>
                {i > 0 && ', '}
                {/* Item deep-links to the KB when the resolver produced a
                    slug; otherwise EntityLink renders the name as plain
                    text. `label` is pinned so the resolved name is never
                    rewritten by the class-id prettifier. */}
                <EntityLink
                  category={p.category}
                  classKey={p.class}
                  label={p.label}
                  resolvedLabel={p.label}
                  resolvedSlug={p.slug}
                />
              </React.Fragment>
            ))}
            {data.hasMoreClasses && ' …'}
          </p>
        )}
        <p className="hud-note">
          <Link href={'/me/loadout' as Route}>View loadout →</Link>
        </p>
      </div>
    );
  },
});
