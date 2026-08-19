import React from 'react';
import Link from 'next/link';
import type { Route } from 'next';
import type { WidgetDef } from './types';

/**
 * `entities` — a compact nav card to the full cross-session entity rollup
 * (`/u/[handle]/entities`). No data of its own; it's the "see everything"
 * affordance. Kept deliberately short (the tile IS the card — no nested box)
 * so it fits without scroll or wasted space.
 */
export const entitiesWidget: WidgetDef = {
  id: 'entities',
  defaultSize: 'compact',
  eyebrow: 'Cross-session rollups',
  isAvailable(ctx) {
    return ctx.isOwner;
  },
  async render(ctx, _size) {
    const href = `/u/${encodeURIComponent(ctx.ownerHandle)}/entities` as Route;
    return (
      <div>
        <p className="hud-note" style={{ margin: 0 }}>
          Ships, weapons, locations &amp; items — aggregated across every session.
        </p>
        <p className="hud-note" style={{ marginTop: 8 }}>
          <Link href={href} data-testid="entities-nav-card">
            Browse entities →
          </Link>
        </p>
      </div>
    );
  },
};
