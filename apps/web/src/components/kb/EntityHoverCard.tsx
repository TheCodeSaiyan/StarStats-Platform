'use client';

/**
 * Small popover surfaced by `EntityLink` on hover. Renders the
 * curated `summary` fields the listing endpoint already shipped, so
 * no per-hover fetch is needed — the data is already in the
 * component tree.
 *
 * The card is `position: fixed` and rendered into `document.body` by
 * `EntityLink`, which measures the link and passes the coordinates in
 * as `pos`. It used to be an absolutely-positioned child of the link,
 * and inside a widget tile that made it invisible: `.hud-tile`
 * (overflow:hidden), `.hud-tile__body` (overflow-y:auto) and
 * `.ss-main` all clipped it — the same defect `InfoTip` had. Until
 * `pos` is known the card renders hidden so it can be measured without
 * flashing at (0,0). Width is fixed (~260px) so multi-row hovers
 * don't shift layout; long values wrap inside their cell.
 *
 * Field set per category mirrors `build_summary` in
 * `crates/starstats-server/src/reference_data.rs`. Update both
 * sides together when the field set changes.
 */

import React, { forwardRef } from 'react';
import {
  type ReferenceCategory,
  type ReferenceEntry,
  prettyItemType,
  tierLabel,
  subtypeLabel,
  placementLabel,
} from '@/lib/reference-types';

/** Viewport coordinates for the card's top-left corner. */
export interface HoverCardPos {
  top: number;
  left: number;
}

/** Fixed card width; `EntityLink` clamps placement against it. */
export const HOVER_CARD_WIDTH = 260;

interface EntityHoverCardProps {
  category: ReferenceCategory;
  entry: ReferenceEntry;
  /** Stable id so the triggering EntityLink can `aria-describedby` it. */
  id?: string;
  /** Measured viewport position. `null` until the first layout pass. */
  pos: HoverCardPos | null;
}

interface Field {
  label: string;
  value: string;
}

/** Per-category projection of the entry's `summary` into ordered
 *  label/value pairs for display. Field order is intentional — most
 *  identifying field first. Discriminates on the summary's tagged
 *  union so each branch sees a fully-narrowed struct. */
function fieldsFor(_category: ReferenceCategory, entry: ReferenceEntry): Field[] {
  const out: Field[] = [];
  const push = (label: string, value: string | undefined) => {
    if (value && value.length > 0) out.push({ label, value });
  };
  const s = entry.summary;
  switch (s.category) {
    case 'vehicle':
      push('Manufacturer', s.manufacturer);
      push('Role', s.role);
      push('Size', s.hull_size);
      push('Focus', s.focus);
      break;
    case 'weapon':
      push('Manufacturer', s.manufacturer);
      push('Type', s.weapon_type);
      push('Size', s.size);
      push('Damage', s.damage_type);
      break;
    case 'item':
      push('Manufacturer', s.manufacturer);
      // Same treatment as the detail page's "Item type" row — the two
      // are deliberately kept in sync. No metadata here, so the token
      // is prettified rather than swapped for classification_label.
      push('Type', s.item_type ? prettyItemType(s.item_type) : undefined);
      push('Grade', s.grade);
      break;
    case 'location':
      // Wave 2 taxonomy first — these answer "what is this place?"
      // more precisely than the Wave 1 classification text. The Wave
      // 1 row stays as a fallback for entries the enrichment cron
      // hasn't matched yet.
      if (s.tier) push('Tier', tierLabel(s.tier));
      if (s.subtype) push('Subtype', subtypeLabel(s.subtype));
      if (s.placement) push('Placement', placementLabel(s.placement));
      push('System', s.system);
      push('Parent', s.parent);
      // Suppress Wave 1 'Type' when the richer Wave 2 subtype is
      // present — they convey overlapping info and stacking both
      // ("Type: City" right under "Subtype: City") is just noise.
      if (!s.subtype) push('Type', s.classification);
      push('Operator', s.operator);
      push('Faction', s.faction);
      push('Tag', s.tag);
      break;
  }
  return out;
}

export const EntityHoverCard = forwardRef<HTMLSpanElement, EntityHoverCardProps>(
  function EntityHoverCard({ category, entry, id, pos }, ref) {
    const fields = fieldsFor(category, entry);
    // Everything here is phrasing content (spans, not <dl>/<dt>/<dd> or
    // <div>): the card is portaled to <body> now, but it is still
    // declared inside EntityLink's inline <span>, and keeping the
    // content model valid costs nothing (M-W10). The CSS grid +
    // `display:contents` rows reproduce the label/value table
    // visually. The trigger wires `aria-describedby` to this `id`, so
    // screen readers still announce the detail on focus.
    return (
      <span
        ref={ref}
        id={id}
        role="tooltip"
        aria-label={`${entry.display_name} details`}
        style={{
          position: 'fixed',
          top: pos?.top ?? 0,
          left: pos?.left ?? 0,
          visibility: pos ? 'visible' : 'hidden',
          width: HOVER_CARD_WIDTH,
          padding: '10px 12px',
          background: 'var(--bg-elev)',
          border: '1px solid var(--border)',
          borderRadius: 0,
          boxShadow: '0 6px 24px rgba(0,0,0,0.25)',
          // Above the tile grid and the sticky chrome, matching
          // `.infotip__pop`.
          zIndex: 200,
          fontSize: 12,
          lineHeight: 1.45,
          color: 'var(--fg)',
          pointerEvents: 'none',
        }}
      >
        <span
          style={{
            display: 'block',
            fontWeight: 600,
            fontSize: 13,
            marginBottom: 6,
          }}
        >
          {entry.display_name}
        </span>
        {fields.length === 0 ? (
          <span style={{ color: 'var(--fg-dim)' }}>No metadata available.</span>
        ) : (
          <span
            style={{
              display: 'grid',
              gridTemplateColumns: 'minmax(70px, max-content) 1fr',
              columnGap: 10,
              rowGap: 4,
            }}
          >
            {fields.map((f) => (
              <span key={f.label} style={{ display: 'contents' }}>
                <span style={{ color: 'var(--fg-muted)', fontSize: 11 }}>
                  {f.label}
                </span>
                <span>{f.value}</span>
              </span>
            ))}
          </span>
        )}
      </span>
    );
  },
);
