import React from 'react';
import { Plane, HoloKV } from 'holo';
import type { DetailGroup } from '@/lib/kb-detail';

/**
 * The curated, grouped metadata sections, drawn in the projection.
 *
 * REDRAWN, not reframed. This used to be a stack of `.ss-card` sections with
 * inline styles — rounded, filled, its own type scale — and the projection port
 * left it alone behind a compatibility rule that squared off its corners. That
 * is how the entity sheet ended up looking like the old page in a new box.
 *
 * Each group is now a `Plane` with the group title as its tracked caption, and
 * the label/value pairs are a `HoloKV` rather than a hand-built `<dl>` grid.
 * The data, the grouping and the order are untouched: `buildDetailGroups` still
 * decides what appears and in what sequence.
 *
 * The title stays a REAL HEADING inside the caption. `Plane`'s `cap` is a
 * styled span, so passing a bare string silently drops the group structure out
 * of the heading outline — which it did, and a spec caught it. The caption's
 * type is reset for `h2`–`h4` so an actual heading looks identical.
 */
export function DetailGroups({ groups }: { groups: DetailGroup[] }) {
  if (groups.length === 0) return null;
  return (
    <>
      {groups.map((group) => (
        <Plane
          key={group.title}
          tilt="flat"
          cap={<h3>{group.title}</h3>}
          style={{ marginTop: 16 }}
        >
          <HoloKV
            items={group.rows.map((row) => ({ k: row.label, v: row.value }))}
          />
        </Plane>
      ))}
    </>
  );
}
