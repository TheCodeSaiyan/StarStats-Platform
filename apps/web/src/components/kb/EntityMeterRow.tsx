'use client';

/**
 * A projection `MeterRow` that carries an entity hover card.
 *
 * The flat widget tiles rendered entity names through `EntityLink`, which
 * brings the hover card with it. The projection's ranked planes render a
 * `MeterRow` where the WHOLE ROW is the anchor, so an `EntityLink` inside one
 * would nest anchors — `MeterRow`'s own comment records that an inner anchor
 * and a stretched overlay were both tried and both failed. The consequence was
 * that the projection had no hover cards at all, and `/me` has been without
 * them since the port; the public profile kept them only for as long as it
 * kept the flat canvas.
 *
 * So the ROW is the trigger and the card is portalled to `document.body` —
 * `.hp-plane` clips, exactly as `.hud-tile` did, so the card can never be a
 * descendant. The open/measure/track/dismiss machinery is `useHoverCard`,
 * shared verbatim with `EntityLink`.
 */

import React from 'react';
import { createPortal } from 'react-dom';
import { MeterRow } from 'holo';
import type { ReferenceCategory, ReferenceEntry } from '@/lib/reference-types';
import { EntityHoverCard } from './EntityHoverCard';
import { useHoverCard } from './useHoverCard';

export interface EntityMeterRowProps {
  rank: number;
  name: React.ReactNode;
  value: string;
  pct: number;
  href?: string;
  category: ReferenceCategory;
  entry: ReferenceEntry;
  /** The projection's row link component (a Next <Link>). */
  linkAs?: React.ElementType;
}

export function EntityMeterRow({
  rank,
  name,
  value,
  pct,
  href,
  category,
  entry,
  linkAs,
}: EntityMeterRowProps) {
  const { hovered, setHovered, cardId, anchorRef, cardRef, mounted } =
    useHoverCard<HTMLElement, HTMLSpanElement>();

  const card = hovered ? (
    <EntityHoverCard
      ref={cardRef}
      id={cardId}
      category={category}
      entry={entry}
    />
  ) : null;

  return (
    <>
      <MeterRow
        rank={rank}
        name={name}
        value={value}
        pct={pct}
        href={href}
        linkAs={linkAs}
        ref={anchorRef}
        onMouseEnter={() => setHovered(true)}
        onMouseLeave={() => setHovered(false)}
        onFocus={() => setHovered(true)}
        onBlur={() => setHovered(false)}
        aria-describedby={card ? cardId : undefined}
      />
      {/* A fragment, not a wrapper: the plane's rows are grid children and an
          extra element between them would break the row rhythm. The card does
          not need to be a sibling — it is portalled to the body. */}
      {card && (mounted ? createPortal(card, document.body) : card)}
    </>
  );
}
