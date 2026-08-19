import Link from 'next/link';
import type { ResolvedLocation, SupporterStatusDto } from '@/lib/api';
import type { ReferenceCatalog } from '@/lib/reference-types';
import { CompassStar } from '@/components/CompassStar';
import { LocationChip } from '@/components/LocationPill';
import { ThemeToggle } from '@/components/theme/ThemeToggle';
import { DrawerToggle } from './DrawerToggle';
import { AccountMenu } from './AccountMenu';
import { RoutePlacard } from './RoutePlacard';

interface Props {
  handle: string | null;
  /**
   * Most recent in-game location, or null when the server reported
   * no recent activity (204) or the fetch failed. Surfaced as a
   * compact chip beside the brand so the user always knows their
   * current grounding without leaving the page.
   */
  location?: ResolvedLocation | null;
  /**
   * ISO 8601 timestamp the user first entered the current location.
   * Drives the chip's "here for X" dwell tail. Null/undefined hides
   * the dwell metric without affecting the rest of the chip — the
   * caller's trace fetch may have failed, or the user may not yet
   * have an active run.
   */
  dwellStart?: string | null;
  /**
   * True when the server marked `entered_at` as a lower bound (the
   * server's walk-back exhausted its event batch without seeing a
   * location change). The chip renders a trailing `+` on the dwell
   * label so the user knows their actual time-at-location is at
   * least that long. Surfaced separately from `dwellStart` so the
   * chip doesn't need to round-trip through the parent layout to
   * know whether to add the marker.
   */
  dwellIsLowerBound?: boolean;
  /** Optional locations catalog. When supplied, the chip's location
   *  text links to `/kb/location/{slug}` with the EntityHoverCard.
   *  Pass from the layout's `getCategoryBundle('location').catalog`. */
  locationCatalog?: ReferenceCatalog;
  /**
   * Caller's own supporter status. When state is `active` or
   * `lapsed`, a compact `<SupporterChip size="sm">` renders next to
   * the handle pill so the user sees their recognition on every
   * page. `null` (default) hides the chip — same posture as the
   * existing handle / location chip pattern. Sourced at the layout
   * level (`Promise.allSettled` with the other shell fetches) so
   * the TopBar stays presentational.
   */
  supporter?: SupporterStatusDto | null;
  /**
   * Site-wide staff grants for the current user. Forwarded to
   * `AccountMenu` so the Admin link appears when appropriate.
   */
  staffRoles: string[];
  /**
   * Number of inbound shares. Forwarded to `AccountMenu` for the
   * badge on "Shared with me". Optional — undefined or 0 hides it.
   */
  inboundShareCount?: number;
}

/**
 * Top app bar — brand mark + location chip + account menu. Sticky-positioned
 * (see `.ss-topbar` in `starstats-tokens.css`) so it stays visible while the
 * main scroll container moves underneath. Mobile renders the drawer toggle on
 * the left; tablet/desktop hide it via tokens.css.
 */
export function TopBar({
  handle,
  location = null,
  dwellStart = null,
  dwellIsLowerBound = false,
  locationCatalog,
  supporter = null,
  staffRoles,
  inboundShareCount,
}: Props) {
  return (
    <header className="ss-topbar">
      <DrawerToggle />
      <Link
        href="/me"
        className="ss-mark"
        style={{ textDecoration: 'none' }}
      >
        <span
          className="ss-mark-glyph mono"
          style={{ color: 'var(--accent)' }}
        >
          <CompassStar size={14} />
        </span>
        <span>STARSTATS</span>
      </Link>
      <RoutePlacard />
      {/*
        Spacer sits BETWEEN the route placard and the location chip so the
        placard's variable width (per-route path string) is absorbed here
        instead of shoving the chip sideways as you navigate. The chip then
        right-anchors against the account menu — a stable readout position
        regardless of route. See docs/ENGINEERING.md `.ss-topbar` flex-order note.
      */}
      <span style={{ flex: 1 }} />
      <LocationChip
        location={location}
        dwellStart={dwellStart}
        dwellIsLowerBound={dwellIsLowerBound}
        catalog={locationCatalog}
      />
      <ThemeToggle />
      <AccountMenu
        handle={handle}
        staffRoles={staffRoles}
        inboundShareCount={inboundShareCount}
        supporter={supporter}
      />
    </header>
  );
}
