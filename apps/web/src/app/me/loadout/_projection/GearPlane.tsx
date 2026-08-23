import React from 'react';
import { Plane } from 'holo';
import type { ResolvedItem } from '@/lib/api';
import { LoadoutItem } from './LoadoutItem';

export interface GearItem {
  cls: string;
  port: string;
  resolved?: ResolvedItem;
}

/**
 * One carried-gear group as a flat Plane.
 *
 * Renders NOTHING when the group is empty — an empty "Throwables" heading
 * says less than no heading at all, and the flat original behaved the same
 * way.
 */
export function GearPlane({
  title,
  items,
}: {
  title: string;
  items: GearItem[];
}) {
  if (items.length === 0) return null;
  return (
    <Plane
      tilt="flat"
      // An `<h3>`, not a bare caption. These were `<h2>`s on the flat page, and
      // dropping them to plain text would have removed the group names from
      // heading navigation entirely — a screen-reader user could no longer jump
      // to "Weapons". The Pane's own title is the h2, so the group is an h3 and
      // the hierarchy stays honest. Styling is unchanged; only the tag.
      cap={<h3>{title}</h3>}
      hint={`${items.length}`}
      style={{ marginTop: 18 }}
    >
      <div className="hp-geargrid">
        {items.map((item) => (
          <LoadoutItem
            key={`${item.cls}__${item.port}`}
            cls={item.cls}
            port={item.port}
            resolved={item.resolved}
          />
        ))}
      </div>
    </Plane>
  );
}
