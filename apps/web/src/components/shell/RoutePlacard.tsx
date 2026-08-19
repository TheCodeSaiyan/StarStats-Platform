'use client';

import { usePathname } from 'next/navigation';

/**
 * Command-bar placard route path (bridge). Renders the ACTUAL current
 * route as an engraved placard beside the wordmark — e.g. `HOME`,
 * `ORGS // NEW`, `KB // VEHICLES`, `PROFILE // THECODESAIYAN`. Uppercasing
 * is done by `.ss-placard` (text-transform), so labels are passed in
 * title case. Never renders a theme name. Decorative wayfinding, so it's
 * aria-hidden — the real navigation lives in the rail + account menu.
 */
const SEGMENT_LABELS: Record<string, string> = {
  me: 'Home',
  loadout: 'Loadout',
  discover: 'Discover',
  orgs: 'Orgs',
  new: 'New',
  devices: 'Devices',
  sharing: 'Sharing',
  preview: 'Preview',
  settings: 'Settings',
  'widget-sharing': 'Widgets',
  '2fa': 'Security',
  submissions: 'Submissions',
  support: 'Support',
  kb: 'Knowledge base',
  vehicle: 'Vehicles',
  weapon: 'Weapons',
  item: 'Items',
  location: 'Locations',
  contracts: 'Contracts',
  admin: 'Admin',
  users: 'Users',
  audit: 'Audit',
  reference: 'Reference',
  'ship-matrix': 'Ship matrix',
  smtp: 'SMTP',
  'parser-submissions': 'Parser shapes',
  u: 'Profile',
  entities: 'Entities',
  sessions: 'Sessions',
  roadmap: 'Roadmap',
  changelog: 'Changelog',
};

function labelForSegment(seg: string): string {
  if (SEGMENT_LABELS[seg]) return SEGMENT_LABELS[seg];
  // Dynamic segment (slug / handle / id): skip opaque ids, humanise slugs.
  if (/^[0-9a-f]{8,}$/i.test(seg) || /^[0-9a-f-]{16,}$/i.test(seg)) return '';
  return seg.replace(/-/g, ' ');
}

export function placardForPath(pathname: string): string | null {
  const segs = pathname.split('/').filter(Boolean);
  if (segs.length === 0) return null;
  const parts: string[] = [];
  for (const seg of segs.slice(0, 3)) {
    const label = labelForSegment(seg);
    if (label) parts.push(label);
  }
  if (parts.length === 0) return null;
  return parts.join(' // ');
}

export function RoutePlacard() {
  const pathname = usePathname();
  const label = pathname ? placardForPath(pathname) : null;
  if (!label) return null;
  return (
    <span
      className="ss-placard ss-route-placard"
      aria-hidden="true"
      style={{ color: 'var(--fg-dim)' }}
    >
      {label}
    </span>
  );
}
