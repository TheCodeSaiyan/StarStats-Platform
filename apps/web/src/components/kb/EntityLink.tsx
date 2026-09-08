'use client';

/**
 * Renders an entity name as either:
 *   - a hover-revealable `<Link>` to `/kb/{category}/{slug}`, when
 *     the catalog has a resolved slug for the class identifier; or
 *   - a plain `<span>`, when the catalog has no entry (or the entry
 *     has no slug yet — legacy rows pre-dating the KB-v1 backfill).
 *
 * The component is a CLIENT component because hover state is needed
 * for the popover trigger. Inside RSC pages, callers wrap an
 * `EntityLink` boundary; the server component supplies the
 * `catalog` prop from a pre-resolved fetch (see
 * `loadAllReferenceBundles`) so the client doesn't have to refetch.
 *
 * Graceful degradation: if `catalog` is missing or the lookup
 * misses, the prettifier still produces a readable string via the
 * existing heuristic (`toFriendlyName`) — the same behaviour
 * `prettyClass` provides in non-React render paths.
 *
 * WHY THE HOVER CARD IS PORTALED
 * ------------------------------
 * The card used to be an absolutely-positioned child of the link's
 * wrapper. Inside a widget tile it opened correctly and was still
 * invisible: `.hud-tile` (overflow:hidden), `.hud-tile__body`
 * (overflow-y:auto) and the row's `.hud-trunc` all clipped it — the
 * same defect `InfoTip` had. Opening those containers is NOT the fix
 * (hud.css records that clipping silently swallowed travel's
 * routes+map once, so the overflow stays). The card leaves the
 * container instead: it renders into `document.body` with
 * `position: fixed` at coordinates measured from the link, and is
 * re-placed on scroll and resize because `fixed` does not follow its
 * anchor.
 */

import React, {
  useCallback,
  useEffect,
  useId,
  useLayoutEffect,
  useRef,
  useState,
} from 'react';
import { createPortal } from 'react-dom';
import Link from 'next/link';
import type { Route } from 'next';
import type {
  ReferenceCatalog,
  ReferenceCategory,
} from '@/lib/reference-types';
import { resolveReferenceEntry } from '@/lib/reference-types';
import { toFriendlyName } from '@/lib/heuristic-name';
import { EntityHoverCard, HOVER_CARD_WIDTH, type HoverCardPos } from './EntityHoverCard';
import { TierChip } from './TierChip';

/** Gap between the link text and the card edge. */
const GAP = 6;
/** Minimum distance the card keeps from any viewport edge. */
const PAD = 8;

interface EntityLinkProps {
  /** Which catalog to look the raw identifier up in. */
  category: ReferenceCategory;
  /** The raw class identifier from the event payload (e.g.
   *  `"AEGS_Avenger_Stalker"`). Case-insensitive lookup. */
  classKey: string | null | undefined;
  /** Catalog for this category. Pass the matching `catalogs.vehicles`
   *  / `catalogs.weapons` / etc. from a `loadAllReferenceBundles()`
   *  resolution. Optional — when omitted, the component always
   *  renders the heuristic fallback. */
  catalog?: ReferenceCatalog;
  /** Override the displayed string. Defaults to the catalog's
   *  `display_name` (or the heuristic). Useful when the caller has
   *  already resolved a label via a different code path. */
  label?: string;
  /** When `category === 'location'` and the resolved entry carries
   *  a taxonomy v2 tier, render a `<TierChip>` immediately after the
   *  link text. Opt-in — dense surfaces (chain strip, timeline
   *  rows) leave it off to avoid chip clutter; hero/pill/topbar
   *  surfaces enable it. */
  showTier?: boolean;
  /** Variant of the tier chip when `showTier` is enabled. `compact`
   *  hides the tier label and renders only the subtype (useful in
   *  contexts where the parent text already conveys the tier). */
  tierChipVariant?: 'full' | 'compact';
  /** Pre-resolved KB slug from an out-of-band classifier — specifically
   *  the tray's fuzzy location resolver, shipped on an event's
   *  `resolved_location`. When present and non-empty it takes
   *  precedence over the catalog lookup for the link target: this is
   *  how a fuzzy-matched location with NO exact catalog key still links
   *  to `/kb/{category}/{slug}`. Falls through to the catalog's slug
   *  when absent. */
  resolvedSlug?: string | null;
  /** Display text paired with `resolvedSlug` — the resolver's
   *  `display_name`. Wins over `label` and the catalog's display name
   *  when provided. */
  resolvedLabel?: string | null;
}

export function EntityLink({
  category,
  classKey,
  catalog,
  label,
  showTier = false,
  tierChipVariant = 'full',
  resolvedSlug,
  resolvedLabel,
}: EntityLinkProps) {
  const [hovered, setHovered] = useState(false);
  const [pos, setPos] = useState<HoverCardPos | null>(null);
  // Stable id to wire the trigger's `aria-describedby` to the hover
  // card so screen-reader users get the detail on focus, not just
  // sighted hover (M-W10).
  const cardId = useId();
  const anchorRef = useRef<HTMLSpanElement>(null);
  const cardRef = useRef<HTMLSpanElement>(null);

  // Portal target only exists on the client. Gate on a mounted flag
  // rather than `typeof document`, so the server render and the first
  // client render agree and hydration doesn't mismatch.
  const [mounted, setMounted] = useState(false);
  useEffect(() => setMounted(true), []);

  const place = useCallback(() => {
    const anchor = anchorRef.current;
    const card = cardRef.current;
    if (!anchor || !card) return;
    const a = anchor.getBoundingClientRect();
    const c = card.getBoundingClientRect();
    const vw = document.documentElement.clientWidth;
    const vh = document.documentElement.clientHeight;
    const width = c.width || HOVER_CARD_WIDTH;

    // Left-align with the link text, then pull back inside the
    // viewport. Clamping the low end last matters: on a narrow screen
    // the card can be wider than the space available, and pinning the
    // left edge beats losing the start of every value.
    let left = a.left;
    left = Math.min(left, vw - PAD - width);
    left = Math.max(PAD, left);

    // Prefer below the link (where it always was); flip above when
    // there isn't room, which is the common case for rows at the
    // bottom of a scrolled tile or the page.
    let top = a.bottom + GAP;
    if (top + c.height > vh - PAD) {
      top = Math.max(PAD, a.top - c.height - GAP);
    }

    setPos({ top: Math.round(top), left: Math.round(left) });
  }, []);

  // Measure before paint so the card never shows at a stale position.
  useLayoutEffect(() => {
    if (!hovered) {
      setPos(null);
      return;
    }
    place();
  }, [hovered, place]);

  // `position: fixed` does not follow the anchor, so track it while
  // open. Capture phase catches scrolls in the tile body and
  // `.ss-main`, not just the window.
  useEffect(() => {
    if (!hovered) return;
    const onMove = () => place();
    window.addEventListener('scroll', onMove, true);
    window.addEventListener('resize', onMove);
    return () => {
      window.removeEventListener('scroll', onMove, true);
      window.removeEventListener('resize', onMove);
    };
  }, [hovered, place]);

  // Escape dismisses the card without moving the pointer (WCAG 1.4.13,
  // content-on-hover-or-focus dismissable) — M-W10. Listened for on
  // the document: a pointer hover never gives the link focus, so a
  // handler on the wrapper would only ever fire for keyboard users.
  useEffect(() => {
    if (!hovered) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') setHovered(false);
    };
    document.addEventListener('keydown', onKey);
    return () => document.removeEventListener('keydown', onKey);
  }, [hovered]);

  // A tray-resolved label can stand in even when there's no classKey
  // (a fuzzy match produced a display name from a raw the catalog
  // doesn't key). Prefer it over an empty span.
  if (!classKey) {
    return <span>{resolvedLabel ?? label ?? ''}</span>;
  }

  const entry = resolveReferenceEntry(category, classKey, catalog);
  const text =
    resolvedLabel ?? label ?? entry?.display_name ?? toFriendlyName(classKey);

  // The tray-resolved slug wins over the catalog lookup — it's the
  // whole point of `resolved_location`: a fuzzy-matched location with
  // no exact catalog key still gets a working `/kb/location/{slug}`
  // link. Empty string is treated as "no slug" (placeless / fallback).
  const effectiveSlug =
    resolvedSlug && resolvedSlug.length > 0 ? resolvedSlug : entry?.slug;

  // Tier chip is opt-in via `showTier` and only meaningful for
  // location entries with Wave 2 taxonomy populated. Render after
  // the link/text body so it sits to the right at baseline.
  const tierNode =
    showTier &&
    entry?.summary.category === 'location' &&
    entry.summary.tier ? (
      <>
        {' '}
        <TierChip
          tier={entry.summary.tier}
          subtype={entry.summary.subtype}
          compact={tierChipVariant === 'compact'}
        />
      </>
    ) : null;

  // Without a slug (neither tray-resolved nor catalog), there's no
  // detail URL to link to. Fall back to plain text — better than a
  // dead link.
  if (!effectiveSlug) {
    return (
      <span>
        {text}
        {tierNode}
      </span>
    );
  }

  // Encode the slug: `resolvedSlug` can be a tray-resolved fuzzy value
  // (not a clean catalog key), so a stray space / `#` / `?` / slash
  // would otherwise corrupt the href. `category` is a fixed union, safe
  // as-is. (L3)
  const href = `/kb/${category}/${encodeURIComponent(effectiveSlug)}` as Route;

  // Hover card needs a catalog entry for its detail fields; a
  // tray-resolved-only link (fuzzy match, no catalog entry) links
  // fine but has nothing to populate the card, so skip it.
  const card =
    hovered && entry ? (
      <EntityHoverCard
        ref={cardRef}
        id={cardId}
        category={category}
        entry={entry}
        pos={pos}
      />
    ) : null;

  return (
    <span
      ref={anchorRef}
      style={{ position: 'relative', display: 'inline-block' }}
      onMouseEnter={() => setHovered(true)}
      onMouseLeave={() => setHovered(false)}
      onFocus={() => setHovered(true)}
      onBlur={() => setHovered(false)}
    >
      <Link
        href={href}
        aria-describedby={card ? cardId : undefined}
        // Disable viewport prefetch: EntityLink is rendered in bulk on
        // feeds, timelines, and dashboards (dozens per page), and each
        // prefetch runs a KB detail SSR render that hits the per-IP
        // rate-limited reference API. Prefetching all of them at once
        // trips the governor (429) and crashes the prefetched page. The
        // KB routes have loading.tsx skeletons, so click navigation
        // still feels instant.
        prefetch={false}
        style={{
          color: 'var(--accent)',
          textDecoration: 'none',
          borderBottom: '1px dotted var(--accent)',
        }}
      >
        {text}
      </Link>
      {tierNode}
      {card && (mounted ? createPortal(card, document.body) : card)}
    </span>
  );
}
