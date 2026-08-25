/**
 * The `/me` "lens" — a client-side focus control that filters the home
 * widget canvas to one dimension (the job the old /journey tabs did).
 * Lens membership is static per widget id, so this map is safe to import
 * into the client canvas component (no server widget defs needed).
 */
import type { WidgetId } from '@/app/_components/widgets/types';

export type Lens =
  | 'all'
  | 'activity'
  | 'travel'
  | 'combat'
  | 'loadout'
  | 'commerce';

export const LENSES: readonly { id: Lens; label: string }[] = [
  { id: 'all', label: 'All' },
  { id: 'activity', label: 'Activity' },
  { id: 'travel', label: 'Travel' },
  { id: 'combat', label: 'Combat' },
  { id: 'loadout', label: 'Loadout' },
  { id: 'commerce', label: 'Commerce' },
];

/** Which lens(es) each widget belongs to.
 *
 *  EXHAUSTIVE on purpose. This was a `Partial` record, and the docstring
 *  here used to state the consequence plainly: "tsc won't catch a missing
 *  entry for a new widget id; adding one here is a manual step for every
 *  new widget." That step was missed seven times out of twenty-one — most
 *  visibly `journey`, the route map, which vanished under the TRAVEL lens.
 *
 *  An empty list is a DECISION and reads as one: the widget spans every
 *  dimension (or carries no data of its own), so it belongs only to `all`.
 *  A NEW widget can no longer arrive without someone choosing. */
export const WIDGET_LENSES: Record<WidgetId, Lens[]> = {
  heatmap: ['activity'],
  sessions: ['activity'],
  recent_activity: ['activity'],
  travel: ['travel'],
  routes: ['travel'],
  locations: ['travel'],
  corridors: ['travel'],
  // Derived from travel telemetry by their own definitions: `journey` is
  // the location trail as a route map, `fleet` ranks vehicles by
  // quantum-travel trips, `docking` counts stow events at the end of one.
  journey: ['travel'],
  fleet: ['travel'],
  docking: ['travel'],
  combat_mission: ['combat'],
  objectives: ['combat'],
  contracts: ['combat'],
  lives: ['combat'],
  loadout: ['loadout'],
  hangar: ['loadout'],
  economy: ['commerce'],
  spend: ['commerce'],
  // Cross-cutting: `records` spans sessions, trades and deaths at once;
  // `orgs` is RSI membership, not a play dimension; `entities` is a nav
  // card with no data of its own. All three belong under `all` only.
  // Facts observe across travel, combat and session rhythm at once, so
  // they belong to no single dimension — `all` only, like `records`.
  facts: [],
  // Client health is a property of PLAYING, not of a play dimension — it
  // belongs beside sessions and the heatmap rather than under combat or
  // travel.
  stability: ['activity'],
  records: [],
  orgs: [],
  entities: [],
};

/** True if the widget should show under the active lens. `all` always
 *  matches; otherwise the widget's lens list must include `lens`. */
export function widgetMatchesLens(id: string, lens: Lens): boolean {
  if (lens === 'all') return true;
  const lenses = WIDGET_LENSES[id as WidgetId];
  return lenses ? lenses.includes(lens) : false;
}
