import React from 'react';
import type { Route } from 'next';
import { getLocationsVisited } from '@/lib/api';
import type { StatsBucket } from '@/lib/api';
import { loadAllReferenceBundles, type ReferenceCatalog } from '@/lib/reference';
import { EntityLink } from '@/components/kb/EntityLink';
import { aggregateLocationBuckets } from '@/lib/class-name-parts';
import { logger } from '@/lib/logger';
import { rangeToHours } from '@/lib/range';
import { defineWidget } from './kit/defineWidget';
import { ReadoutGroup, RankedList } from './kit/archetypes';
import { fmtNum } from './kit/format';
import { InfoTip } from '@/components/hud/InfoTip';
import { INFERENCE_EXPLANATIONS } from '@/lib/inference-explanations';

/**
 * `locations` — "Places visited": distinct-location count plus the top
 * locations by visit count in the active range window
 * (`GET /v1/me/stats/locations?hours=`). Owner-only (me-scoped) and
 * RANGE-AWARE — it re-queries when the dashboard range changes.
 *
 * Migrated to the kit: `ReadoutGroup` carries the distinct-locations
 * headline, `RankedList` caps the ranked list to a top-N with a "See all"
 * link (no scroll). Location keys are `system|planet|city`; the display
 * value wraps in `<EntityLink category="location">` (label pinned for
 * free-text safety). The ranked list keeps its `.hud-secondary` treatment
 * so a squeezed tile drops it before the headline.
 */
interface LocationsData {
  unique: number;
  top: StatsBucket[];
  locations: ReferenceCatalog;
}

export const locationsWidget = defineWidget<LocationsData>({
  id: 'locations',
  eyebrow: 'Places',
  rangeAware: true,
  visibility: 'owner',
  async load(ctx) {
    if (!ctx.token) return null;
    const hours = rangeToHours(ctx.range);
    let top: StatsBucket[] = [];
    let unique = 0;
    try {
      const res = await getLocationsVisited(ctx.token, hours);
      top = res?.top_locations ?? [];
      unique = res?.unique_locations ?? 0;
    } catch (err) {
      logger.warn({ err, call: 'widget.locations' }, 'fetch failed');
      return null;
    }
    if (unique === 0 && top.length === 0) return null;
    const { catalogs } = await loadAllReferenceBundles();
    return { unique, top, locations: catalogs.locations };
  },
  body(data) {
    // Resolve the raw `system|planet|city` keys (and any engine ids) to
    // friendly place names, merging duplicates that collapse to the same
    // label.
    const agg = aggregateLocationBuckets(
      data.top.map((b) => ({ value: b.value, count: b.count })),
    );
    const rows = agg.map((a) => ({
      key: a.label,
      // Deep-link each place to the KB using the FRIENDLY label as the
      // classKey — the catalog is dual-keyed by `display_name`, so real
      // places ("microTech", "New Babbage") resolve and link, while
      // synthetic / merged labels miss and EntityLink degrades to plain
      // text. `label` is pinned so the class-id prettifier never rewrites
      // the friendly place name.
      label: (
        <EntityLink
          category="location"
          classKey={a.label}
          catalog={data.locations}
          label={a.label}
        />
      ),
      value: fmtNum(a.count),
    }));
    return (
      <div className="hud-readout-stack">
        <ReadoutGroup
          readouts={[
            {
              label: 'distinct',
              info: (
                <InfoTip
                  label="distinct places"
                  text={INFERENCE_EXPLANATIONS.distinct_locations}
                />
              ),
              value: fmtNum(data.unique),
            },
          ]}
        />
        <div className="hud-secondary">
          <RankedList
            rows={rows}
            cap={6}
            seeMore={{
              href: '/me/travel' as Route,
              label: (_hidden, total) => `See all ${total.toLocaleString()} →`,
            }}
          />
        </div>
      </div>
    );
  },
});
