/**
 * One nav model, every chrome.
 *
 * The design system is explicit that this is centralised rather than
 * hand-listed per screen: screens that built their own two- or three-link
 * chrome left most of a reader's own areas unreachable from those pages.
 *
 * ACCESS IS PART OF THE MODEL, not a styling concern. A signed-out visitor must
 * not even see the LABELS of pages they cannot open — "Records" and "Calibrate"
 * tell an outsider what exists and invite a bounce off a login wall. So the set
 * is filtered before it is rendered.
 *
 * And hiding a link is presentation, NEVER protection. Every `user` and `admin`
 * route below is independently guarded (middleware + a `getSession()` check in
 * the route itself). This module decides what is offered, not what is allowed.
 *
 * Plain data with no React and no server-only import, so both server chrome and
 * client chrome can read it.
 */
import type { Route } from 'next';

export type NavAccess = 'public' | 'user' | 'admin';

export interface NavDestination {
  /** Stable id, also used to mark the active entry. */
  id: string;
  label: string;
  href: Route;
  access: NavAccess;
}

/**
 * Labels follow the product's voice: in-universe nouns for chrome, plain nouns
 * for anything that can go wrong. Downloads → Emitter, settings → Calibrate.
 *
 * THERE IS NO "HANGAR" ENTRY. There was: it pointed at `/devices`, the paired-
 * device page. That double-booked the word — the actual hangar is the RSI
 * fleet, which is the `hangar` and `fleet` widgets — and it split the emitter's
 * lifecycle across two destinations, so a reader who had just downloaded the
 * tray had to go find a second page to pair it. Pairing, the uplink list and
 * ingest activity all moved into Emitter; `/devices` redirects there and, being
 * a redirect, is not offered as a destination.
 *
 * "StarPlatform", never "OrgPlatform" — the product was renamed and
 * `/org-platform` is a permanent redirect, so the old name is never offered as
 * a destination.
 */
export const SITE_NAV: readonly NavDestination[] = [
  // The public set was originally sized to match what the flat `MarketingNav`
  // offered a signed-out visitor (that component is gone — every signed-out
  // route is a projection now, so it had no render site left). It was four entries at first, which was fine while every
  // projection surface was signed-in — but `/kb` is public, so a visitor
  // reading the catalogue in the projection would have been offered strictly
  // LESS than the same visitor in the flat shell. Verified against the routes:
  // all eight exist.
  //
  // "StarPlatform", never "OrgPlatform" — `/org-platform` is a permanent
  // redirect and is never surfaced as a destination.
  { id: 'home', label: 'Overview', href: '/' as Route, access: 'public' },
  { id: 'features', label: 'Features', href: '/features' as Route, access: 'public' },
  { id: 'star-platform', label: 'StarPlatform', href: '/star-platform' as Route, access: 'public' },
  { id: 'docs', label: 'Docs', href: '/docs' as Route, access: 'public' },
  { id: 'guides', label: 'Guides', href: '/guides' as Route, access: 'public' },
  { id: 'downloads', label: 'Emitter', href: '/downloads' as Route, access: 'public' },
  { id: 'trust', label: 'Trust', href: '/trust' as Route, access: 'public' },
  { id: 'privacy', label: 'Privacy', href: '/privacy' as Route, access: 'public' },
  { id: 'terms', label: 'Terms', href: '/terms' as Route, access: 'public' },

  { id: 'me', label: 'Projection', href: '/me' as Route, access: 'user' },
  // `/me/travel`, NOT `/journey`. `/journey` is a redirect stub to `/me`
  // (superseded by the focus lens), and the system's own rule is that a
  // permanent redirect is never surfaced as a destination — the same reason
  // "OrgPlatform" is never offered. Offering it here put a nav entry in the
  // rail that bounced the reader straight back to the page they were on.
  { id: 'travel', label: 'Travel', href: '/me/travel' as Route, access: 'user' },
  { id: 'contracts', label: 'Contracts', href: '/me/contracts' as Route, access: 'user' },
  { id: 'loadout', label: 'Loadout', href: '/me/loadout' as Route, access: 'user' },
  { id: 'kb', label: 'Catalogue', href: '/kb' as Route, access: 'user' },
  { id: 'discover', label: 'Directory', href: '/discover' as Route, access: 'user' },
  { id: 'sharing', label: 'Sharing', href: '/sharing' as Route, access: 'user' },
  { id: 'settings', label: 'Calibrate', href: '/settings' as Route, access: 'user' },

  { id: 'admin', label: 'Console', href: '/admin' as Route, access: 'admin' },
];

export interface NavOpts {
  signedIn: boolean;
  /**
   * Site-wide staff grants from the session (`Session.staffRoles`). Admin
   * implies moderator server-side, so either grant reaches the console — the
   * same `.some(r => r === 'admin' || r === 'moderator')` check `/admin`
   * gating already uses. Legacy cookies minted before the field existed read
   * as `[]`, which correctly hides the console rather than guessing.
   */
  staffRoles?: readonly string[];
}

/**
 * Which entries the chrome offers in its INLINE row, as opposed to only inside
 * its disclosure menu.
 *
 * The bar and the menu are two different questions and this file used to answer
 * only one. Every destination a session could reach went into both, so a
 * signed-in reader's bar carried seventeen links — nine of them public pages
 * they were not working in — and `ChromeBar`'s fit measurement, which is
 * all-or-nothing, put the whole set behind a hamburger at every viewport up to
 * 2560px. Measured, not guessed: on `/me` the inline row wanted 1953px of which
 * the nav was 687, so a reader on a large desktop navigated by opening a menu.
 *
 * The rule, in the order it is read:
 *
 *   - HOME is always offered. A way back to the front of the site is not a
 *     marketing link, and it is the one public destination a signed-in reader
 *     is known to want.
 *   - A reader's OWN pages are always offered. `user` and `admin` entries only
 *     survive `navFor` when the session may reach them, so this cannot leak a
 *     label to someone who cannot open the page.
 *   - Everything else — features, docs, guides, legal — is offered inline only
 *     while SIGNED OUT, where it is the whole point of the page.
 *
 * Nothing becomes unreachable: the disclosure keeps the full grouped set and
 * stays available even when the inline row fits. That is the difference between
 * hiding a destination and moving it.
 */
export function isPrimaryNav(n: NavDestination, signedIn: boolean): boolean {
  if (n.id === 'home') return true;
  if (n.access !== 'public') return true;
  return !signedIn;
}

/** Which nav entries a session may see. Signed out gets public only. */
export function navFor({ signedIn, staffRoles }: NavOpts): NavDestination[] {
  const isStaff = (staffRoles ?? []).some(
    (r) => r === 'admin' || r === 'moderator',
  );
  return SITE_NAV.filter((n) => {
    if (n.access === 'public') return true;
    if (!signedIn) return false;
    if (n.access === 'admin') return isStaff;
    return true;
  });
}

const NAV_GROUPS: Record<NavAccess, string> = {
  public: 'Site',
  user: 'Your data',
  admin: 'Operator',
};

export interface NavSectionModel {
  title: string;
  items: {
    id: string;
    label: string;
    href: string;
    active?: boolean;
    /** Offered in the chrome's inline row. See `isPrimaryNav`. */
    primary?: boolean;
  }[];
}

/**
 * Grouped for the menu: a flat list of fourteen destinations does not tell a
 * reader which are public pages and which are their own data.
 */
export function navSections(
  opts: NavOpts,
  activeId?: string,
): NavSectionModel[] {
  const out: NavSectionModel[] = [];
  for (const n of navFor(opts)) {
    const title = NAV_GROUPS[n.access];
    let g = out.find((x) => x.title === title);
    if (!g) {
      g = { title, items: [] };
      out.push(g);
    }
    g.items.push({
      id: n.id,
      label: n.label,
      href: n.href,
      active: n.id === activeId,
      primary: isPrimaryNav(n, opts.signedIn),
    });
  }
  return out;
}
