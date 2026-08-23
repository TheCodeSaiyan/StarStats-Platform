import type { SettingsGroup } from './SettingsProjection';

/**
 * The lens-rail groups for `/settings`.
 *
 * These are the categories the scroll-spy sidebar used before the port, in the
 * same order — the sidebar and its `settings-nav-config.ts` retired with the
 * flat page, so this is now the single group axis. Section ids stay the
 * redirect targets and live on each entry in `page.tsx`.
 *
 * Order is document order, so the rail reads the way the page used to scroll:
 * what you set up first, then who you are, then how you are protected, then
 * the thing you cannot undo.
 */
export const SETTINGS_GROUPS: readonly SettingsGroup[] = [
  { key: 'general', label: 'General' },
  { key: 'account', label: 'Account' },
  { key: 'security', label: 'Security' },
  { key: 'danger', label: 'Danger' },
];
