/**
 * In-Transit movement-noise suppression for the tray's friendly activity
 * timeline (StatusPane).
 *
 * These self-explanatory movement events — started moving, switched server,
 * queued a quantum jump, etc. — carry no outcome the player wants in their
 * logbook view. Suppression is DISPLAY-ONLY: the events still persist in the
 * DB and remain visible in the raw Logs view (which has its own type-filter
 * pills). Only the friendly timeline hides them.
 *
 * MUST stay in sync with the equivalent set in
 * `apps/web/src/lib/event-filter.ts` — the "In-Transit hidden in BOTH apps"
 * invariant in docs/ENGINEERING.md. (Moved here from the now-deleted `src/timeline/`
 * module, which was dead code, so the suppression was never actually applied
 * in the tray until M-U1 wired it into StatusPane.)
 */

/** Snake-case `event_type` discriminators the friendly timeline suppresses. */
export const IN_TRANSIT_HIDDEN_TYPES: ReadonlySet<string> = new Set([
  'join_pu',
  'change_server',
  'quantum_target_selected',
  'seed_solar_system',
  'resolve_spawn',
]);

/** True when a snake_case `event_type` is a suppressed movement variant. */
export function isInTransitNoise(eventType: string): boolean {
  return IN_TRANSIT_HIDDEN_TYPES.has(eventType);
}
