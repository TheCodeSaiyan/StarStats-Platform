import type { CatalogueEntry } from 'holo';
import { PROJECTION_CATALOGUE } from '@/app/me/_projection/catalogue';
import type { WidgetId } from '@/app/_components/widgets/types';

/**
 * What an owner can put on their public profile.
 *
 * IDS AND LABELS ARE REUSED, NOT MINTED. They are `WidgetId`s persisted on the
 * account under the `profile` surface, so a new vocabulary would either need a
 * backend change or would silently drop every owner's saved layout. The names
 * and hints come from the `/me` catalogue for the same reason the ids do —
 * calling the same widget two different things on two surfaces is a way to
 * make a reader doubt they are the same widget.
 *
 * ORDER FOLLOWS `DEFAULT_LAYOUT`, which is the profile surface's own list and
 * already decides what a profile carries. This catalogue does not get to add
 * to it: a widget absent there has no place on a profile, and one present
 * there is offered whether or not it is on by default.
 *
 * GROUPS ARE THE PROFILE'S, not `/me`'s. The `/me` catalogue groups by
 * "Callouts" / "Lens panes" / "Centre ring", which describe where an element
 * hangs in a volume that has lenses and a ring driven by the reader's layout.
 * A profile has neither: its ring carries the public `by_type` distribution,
 * and its dock is a flat stack. So the grouping is by what the reader actually
 * sees — a figure in the summary row, or a panel below it.
 *
 * `journey` is DELIBERATELY ABSENT. It is the one element the `/me` catalogue
 * marks `kind: 'ring'`, and this surface's ring is not layout-driven, so
 * enabling it here could never draw anything. The system's own rule is that a
 * control which does nothing is worse than an absent one.
 */
/**
 * MIRRORED FROM `DEFAULT_LAYOUT`, not imported from it.
 *
 * `lib/profile-layout.ts` is `server-only` — it reaches the API — and this
 * module is pulled into the client bundle by the editor, so importing it there
 * fails the build outright. Duplicating the list is the trade, and the drift
 * it invites is closed by `catalogue.test.ts`, which imports both and fails if
 * they ever disagree. The test is the point: without it this is a copy that
 * silently rots the first time a widget is added to the profile surface.
 */
const PROFILE_IDS: readonly WidgetId[] = [
  'sessions', 'heatmap', 'orgs', 'recent_activity', 'combat_mission',
  'economy', 'travel', 'journey', 'records', 'stability', 'hangar',
  'loadout', 'entities', 'objectives', 'spend', 'routes', 'locations',
  'corridors', 'facts',
] as const;

/** On by default on a fresh profile — mirrors `DEFAULT_LAYOUT`'s `enabled`. */
const DEFAULT_ON = new Set<string>(['sessions', 'heatmap', 'orgs', 'entities']);

export const PROFILE_CATALOGUE: readonly CatalogueEntry[] = PROFILE_IDS.flatMap(
  (id) => {
    const el = PROJECTION_CATALOGUE.find((e) => e.id === id);
    // Absent from the projection catalogue, or ring-drawn: nothing on this
    // surface could render it.
    if (!el || el.kind === 'ring') return [];
    return [
      {
        id: el.id,
        name: el.name,
        hint: el.hint,
        group: el.kind === 'callout' ? 'Figures' : 'Panels',
        on: DEFAULT_ON.has(el.id),
      } satisfies CatalogueEntry,
    ];
  },
);

/** The ids this surface knows how to draw — the write filter for the action. */
export const PROFILE_ELEMENT_IDS: readonly string[] = PROFILE_CATALOGUE.map(
  (e) => e.id,
);
