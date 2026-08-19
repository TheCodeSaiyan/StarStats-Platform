/**
 * Single source of truth for the Settings two-pane left-nav (M2).
 *
 * Both the server page (`settings/page.tsx`, which renders the anchored
 * sections) and the client `<SettingsNav>` (scroll-spy sidebar) import
 * this list, so the nav and the content can never drift: every `item.id`
 * here MUST correspond to an `id="…"` on a rendered section in the page,
 * and every anchored section in the page MUST appear here.
 *
 * The `id`s are the EXISTING section anchors — they double as the
 * post-redirect fragments used by the server actions (e.g.
 * `/settings#security`, `/settings#danger`, `/settings#rsi`), so they
 * are behaviour-load-bearing and must not be renamed here without
 * updating the redirects that target them.
 */

export interface SettingsNavItem {
  /** Anchor id — matches the `id` on the section element in the page. */
  readonly id: string;
  /** Sidebar label for this section. */
  readonly label: string;
}

export interface SettingsNavCategory {
  /** React key only — never rendered as a DOM id, so it can't collide
   *  with a section anchor id. */
  readonly key: string;
  /** Category heading shown above its items in the sidebar. */
  readonly label: string;
  readonly items: readonly SettingsNavItem[];
}

export const SETTINGS_NAV: readonly SettingsNavCategory[] = [
  {
    key: 'general',
    label: 'General',
    items: [
      // Appearance is the slot the separate animation-speed program can
      // extend — its wave-speed control already lives inside #theme.
      { id: 'theme', label: 'Appearance' },
      { id: 'timezone', label: 'Local time' },
    ],
  },
  {
    key: 'account',
    label: 'Account',
    items: [
      { id: 'account-info', label: 'Account info' },
      { id: 'verification', label: 'Email verification' },
      { id: 'rsi', label: 'RSI handle' },
      { id: 'hangar', label: 'Device sync' },
      { id: 'email', label: 'Sign-in email' },
    ],
  },
  {
    key: 'security',
    label: 'Security',
    items: [
      { id: 'password', label: 'Password' },
      { id: 'security', label: 'Two-factor' },
    ],
  },
  {
    key: 'danger',
    label: 'Danger zone',
    items: [{ id: 'danger', label: 'Delete account' }],
  },
];

/** Flat, document-order list of every anchor id in the nav. */
export const SETTINGS_NAV_IDS: readonly string[] = SETTINGS_NAV.flatMap(
  (category) => category.items.map((item) => item.id),
);
