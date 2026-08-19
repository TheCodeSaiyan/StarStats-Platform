import type { WidgetId } from '@/app/_components/widgets/types';

/** Column span per widget on the 4-col HUD grid. Span = importance.
 *  Heatmap is the wide hero; travel/combat get room for their bar-lists;
 *  single-readout widgets are 1-wide. */
export const TILE_SPANS: Record<WidgetId, number> = {
  heatmap: 4,
  travel: 2,
  // Journey expands to the route map / timeline / heatmap stack — give it
  // the same 2-col room the travel bar-list gets.
  journey: 2,
  combat_mission: 2,
  sessions: 2,
  recent_activity: 2,
  economy: 1,
  loadout: 1,
  records: 1,
  hangar: 1,
  orgs: 1,
  entities: 1,
  // /me lifetime stat tiles: lives + fleet pair with docking to fill a
  // clean 4-col row (1 + 1 + 2) directly under the heatmap hero.
  lives: 1,
  fleet: 1,
  docking: 2,
  // Correlation / depth widgets (reparse-gated me-scoped aggregates).
  objectives: 1,
  contracts: 1,
  spend: 1,
  routes: 1,
  locations: 1,
  // Corridors is a bar-list like travel/combat, not a single readout —
  // it renders up to 6 `A ⇄ B` rows with weight bars and is enabled
  // EXPANDED on the home layout. At span 1 that content had nowhere to
  // go; it was the only widget in this table sized expanded-at-span-1.
  corridors: 2,
  facts: 2,
};
