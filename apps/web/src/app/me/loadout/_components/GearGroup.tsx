import React from 'react';
import { ItemTile } from './ItemTile';
import type { ResolvedItem } from '@/lib/api';

interface GearGroupItem {
  cls: string;
  port: string;
  resolved?: ResolvedItem;
}

interface GearGroupProps {
  title: string;
  items: GearGroupItem[];
}

/**
 * Titled section containing a grid of ItemTile components.
 * Renders nothing when items is empty.
 */
export function GearGroup({ title, items }: GearGroupProps) {
  if (items.length === 0) return null;

  return (
    <section className="gear-group">
      <h2 className="gear-group__title">{title}</h2>
      <div className="gear-group__grid">
        {items.map((item) => (
          <ItemTile
            key={`${item.cls}__${item.port}`}
            cls={item.cls}
            port={item.port}
            resolved={item.resolved}
          />
        ))}
      </div>
    </section>
  );
}
