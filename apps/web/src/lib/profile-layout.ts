import 'server-only';

import type { LayoutEntry, LayoutSurface } from './api';
import type { WidgetId } from '@/app/_components/widgets/types';

/**
 * Default layout used when the user has never customised. Order
 * mirrors today's hard-coded card order so existing users see no
 * change when the flag flips on. New widgets shipped in Plan 3+
 * will appear in the editor via the GET projection (Phase 3) with
 * `enabled: false` — they don't auto-show.
 */
export const DEFAULT_LAYOUT: LayoutEntry[] = [
  { id: 'sessions', enabled: true, size: 'compact' },
  { id: 'heatmap', enabled: true, size: 'expanded' },
  { id: 'orgs', enabled: true, size: 'compact' },
  { id: 'recent_activity', enabled: false, size: 'compact' },
  { id: 'combat_mission', enabled: false, size: 'compact' },
  { id: 'economy', enabled: false, size: 'compact' },
  { id: 'travel', enabled: false, size: 'compact' },
  // journey is owner-only (me-scoped location endpoints); carried
  // disabled on the public profile so it appears in the editor but never
  // renders for a visitor (isAvailable also gates on ctx.isOwner).
  { id: 'journey', enabled: false, size: 'compact' },
  { id: 'records', enabled: false, size: 'compact' },
  // hangar + loadout ship disabled by default — owners opt-in via the
  // layout editor. Both require tray/parser data that may not exist yet.
  { id: 'hangar', enabled: false, size: 'compact' },
  { id: 'loadout', enabled: false, size: 'compact' },
  { id: 'entities', enabled: true, size: 'compact' },
  // Reparse-gated me-scoped depth widgets — carried disabled so they
  // appear in the editor's Add-widget palette but never render for a
  // visitor (all owner-gated in `isAvailable`).
  { id: 'objectives', enabled: false, size: 'compact' },
  { id: 'spend', enabled: false, size: 'compact' },
  { id: 'routes', enabled: false, size: 'compact' },
  { id: 'locations', enabled: false, size: 'compact' },
  // corridors is owner-only (me-scoped location trace); carried disabled
  // on the public profile so it appears in the editor but never renders
  // for a visitor (isAvailable also gates on ctx.isOwner).
  { id: 'corridors', enabled: false, size: 'compact' },
  // facts are me-scoped observations; never on a visitor's view.
  { id: 'facts', enabled: false, size: 'compact' },
];

/**
 * Default layout for the private /me home surface. Curated to show the
 * activity heatmap plus the range-aware dimension widgets (travel,
 * combat/mission, economy, sessions) enabled out of the box. Other
 * widgets are carried forward disabled so the editor can show them.
 */
export const HOME_DEFAULT_LAYOUT: LayoutEntry[] = [
  { id: 'heatmap', enabled: true, size: 'expanded' },
  // Lifetime character/fleet/docking stat tiles — one grid row under the
  // heatmap. Formerly always-on standalone tiles above the canvas; now
  // editable widgets (owner-only, range-independent).
  { id: 'lives', enabled: true, size: 'compact' },
  { id: 'fleet', enabled: true, size: 'compact' },
  { id: 'docking', enabled: true, size: 'compact' },
  // `routes` takes the slot `travel` held, so the grid keeps its shape.
  // It earns it on four counts: `travel`'s expanded view ALREADY showed
  // "the top routes" and linked to /journey — both separate tiles here,
  // so the pair put the same ranked destinations on the dashboard twice;
  // `routes` carries a period-over-period trend where `travel` shows bare
  // numbers; `travel` reads `/v1/me/metrics/event-types`, the raw
  // event-type presentation widgets-v2 set out to retire; and `routes`
  // degrades to an honest empty-window state where `travel` does not.
  { id: 'routes', enabled: true, size: 'compact' },
  // Journey route map — owner-only, enabled on the private home surface.
  // Expanded by default so the transition graph + timeline show without
  // a resize.
  { id: 'journey', enabled: true, size: 'expanded' },
  // Top corridors — owner-only, enabled on the private home surface.
  // Expanded so the ranked A ⇄ B leaderboard shows without a resize.
  { id: 'corridors', enabled: true, size: 'expanded' },
  // Flight facts — expanded so all three observations show their
  // arithmetic without a resize.
  { id: 'facts', enabled: true, size: 'expanded' },
  { id: 'combat_mission', enabled: true, size: 'compact' },
  { id: 'economy', enabled: true, size: 'compact' },
  { id: 'sessions', enabled: true, size: 'compact' },
  { id: 'recent_activity', enabled: false, size: 'compact' },
  { id: 'records', enabled: false, size: 'compact' },
  { id: 'orgs', enabled: false, size: 'compact' },
  { id: 'hangar', enabled: false, size: 'compact' },
  { id: 'loadout', enabled: false, size: 'compact' },
  { id: 'entities', enabled: false, size: 'compact' },
  // Reparse-gated depth widgets — off by default, available from the
  // Add-widget palette in the editor. `objectives` starts on so the new
  // mission surface is discoverable out of the box.
  { id: 'objectives', enabled: true, size: 'compact' },
  { id: 'contracts', enabled: true, size: 'compact' },
  { id: 'spend', enabled: false, size: 'compact' },
  // Off by default, not removed: still in the editor's Add-widget palette
  // for anyone who wants the quantum-jump / server-hop / planet counts
  // back. Owners with a saved layout are untouched — `projectLayout` uses
  // `stored ?? fallback`, so this default reaches only uncustomised homes.
  { id: 'travel', enabled: false, size: 'compact' },
  { id: 'locations', enabled: false, size: 'compact' },
];

/**
 * Project a stored layout against the registry's known widget ids.
 *
 *  - Unknown ids (widget was renamed / removed) are dropped from the
 *    output.
 *  - Known ids missing from the stored layout are appended at the
 *    end with `enabled: false, size: 'compact'` (so they appear in
 *    the editor in Phase 3 as dim "hidden" strips, without
 *    auto-showing on the profile).
 *
 * Phase 1+2 doesn't render disabled widgets at all (no edit mode),
 * so the appended entries are filtered out at render time. But we
 * project here so the data structure is ready for Phase 3.
 */
export function projectLayout(
  stored: LayoutEntry[] | null | undefined,
  registry: readonly WidgetId[],
  fallback: LayoutEntry[] = DEFAULT_LAYOUT,
): LayoutEntry[] {
  const base = stored ?? fallback;
  const known = base.filter((e) => (registry as readonly string[]).includes(e.id));
  const presentIds = new Set(known.map((e) => e.id));
  // Appended widgets take their SIZE from the curated fallback, and only
  // fall back to `compact` when the fallback does not mention them.
  //
  // Hardcoding `compact` here silently discarded design intent for every
  // widget added after a layout was saved. `corridors` ships as
  // `expanded` precisely because a bare corridor COUNT is not what the
  // tile is for — but existing owners were handed the compact entry,
  // enabled it from the palette, and got the count. Reported as "the
  // corridor widget is still not showing the actual corridors".
  //
  // `enabled` is NOT inherited: a saved layout must never gain widgets on
  // its own. The owner opts in; we just make sure that when they do, they
  // get the size the tile was designed around.
  const fallbackById = new Map(fallback.map((e) => [e.id, e]));
  const missing: LayoutEntry[] = (registry as readonly string[])
    .filter((id) => !presentIds.has(id))
    .map((id) => ({
      id,
      enabled: false,
      size: fallbackById.get(id)?.size ?? ('compact' as const),
    }));
  return [...known, ...missing];
}

import { REGISTERED_IDS } from '@/app/_components/widgets/registry';
import { getProfileLayout } from './api';

/**
 * Server-side fetch + projection used by the profile and /me page render.
 * Falls back to the surface-appropriate default layout when the API call
 * fails — never fails the page over a missing layout column.
 *
 * `surface` defaults to `'profile'` so existing call sites are unchanged.
 */
export async function getProfileLayoutForRender(
  token: string | null,
  _ownerHandle: string,
  isOwner: boolean,
  surface: LayoutSurface = 'profile',
): Promise<LayoutEntry[]> {
  const fallback = surface === 'home' ? HOME_DEFAULT_LAYOUT : DEFAULT_LAYOUT;
  // Visitors don't fetch the owner's layout via the owner-only
  // endpoint. For Phase 1+2, visitors always see the projected
  // DEFAULT_LAYOUT (which matches today's hard-coded order).
  // Phase 3 will add a public-projection endpoint OR a join in the
  // existing friend-summary query.
  if (!token || !isOwner) {
    return projectLayout(null, REGISTERED_IDS, fallback);
  }
  try {
    const res = await getProfileLayout(token, surface);
    return projectLayout(res.layout ?? null, REGISTERED_IDS, fallback);
  } catch {
    return projectLayout(null, REGISTERED_IDS, fallback);
  }
}
