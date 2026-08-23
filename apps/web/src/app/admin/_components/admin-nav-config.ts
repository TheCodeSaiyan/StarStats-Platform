/**
 * Single source of truth for the /admin sub-navigation.
 *
 * `AdminNav` renders these; `admin-nav-config.test.ts` asserts every
 * href resolves to a real page.tsx on disk. That test is deliberately
 * filesystem-backed: deriving the expectation from this file (the way
 * a count test does) would pass even when a nav
 * entry points at a route that no longer exists.
 *
 * Grouping replaces the previous flat 16-pill row. `Settings` collapses
 * the former SMTP / Appearance / Ship Matrix tabs, which now redirect
 * into anchors on /admin/settings.
 */

export interface AdminNavItem {
  /** Stable id, used for the active-state match. */
  readonly id: string;
  /** Sidebar label. */
  readonly label: string;
  /** Route, optionally with a canonical default query string. */
  readonly href: string;
}

export interface AdminNavCategory {
  /** React key only — never rendered as a DOM id. */
  readonly key: string;
  readonly label: string;
  readonly items: readonly AdminNavItem[];
}

export const ADMIN_NAV: readonly AdminNavCategory[] = [
  {
    key: 'overview',
    label: 'Overview',
    items: [{ id: 'dashboard', label: 'Dashboard', href: '/admin' }],
  },
  {
    key: 'people',
    label: 'People',
    items: [
      { id: 'users', label: 'Users', href: '/admin/users' },
      { id: 'orgs', label: 'Orgs', href: '/admin/orgs' },
      { id: 'waitlist', label: 'Waitlist', href: '/admin/waitlist' },
    ],
  },
  {
    key: 'moderation',
    label: 'Moderation',
    items: [
      // Default the queue filter to `review` so the link is canonical
      // ("nothing to triage" lands you on the right bucket).
      {
        id: 'submissions',
        label: 'Submissions',
        href: '/admin/submissions?status=review',
      },
      { id: 'sharing', label: 'Sharing', href: '/admin/sharing' },
      { id: 'audit', label: 'Audit log', href: '/admin/audit' },
    ],
  },
  {
    key: 'parser',
    label: 'Parser',
    items: [
      // Rule-author triage queue for tray-promoted line samples.
      {
        id: 'parser-submissions',
        label: 'Parser shapes',
        href: '/admin/parser-submissions?status=pending',
      },
      // Post-publish retract/re-enable console for the rules published
      // from that queue.
      { id: 'parser-rules', label: 'Parser rules', href: '/admin/parser-rules' },
      // Watches whether shipped matchers still match anything.
      {
        id: 'parser-health',
        label: 'Parser health',
        href: '/admin/parser-health',
      },
      // Rules that infer a synthesized event from a trigger pattern.
      {
        id: 'inference-rules',
        label: 'Inference rules',
        href: '/admin/parser-inference-rules',
      },
      // Run-observed contract names with no published catalog row.
      {
        id: 'contract-gaps',
        label: 'Contract gaps',
        href: '/admin/contract-gaps',
      },
    ],
  },
  {
    key: 'config',
    label: 'Config',
    items: [
      // Collapses the former SMTP / Appearance / Ship Matrix tabs;
      // those routes now redirect to anchors on this page.
      { id: 'settings', label: 'Settings', href: '/admin/settings' },
      { id: 'reference', label: 'Reference', href: '/admin/reference' },
    ],
  },
];

/** Flat, document-order list of every nav item. */
export const ADMIN_NAV_ITEMS: readonly AdminNavItem[] = ADMIN_NAV.flatMap(
  (category) => category.items,
);
