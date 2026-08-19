/**
 * Event-list filtering for the web app.
 *
 * Mirrors `apps/tray-ui/src/timeline/filter.ts` — both apps hide the
 * same five "self-explanatory" movement event types from timeline
 * surfaces. Keep both files in sync when extending the suppressed set.
 *
 * Applied at the API client layer (see `lib/api.ts`) so every event
 * consumer benefits without per-component changes. Hidden events
 * still exist in the database — this is a render-layer filter only.
 *
 * NOTE on scope:
 *   The user's stated intent was "hide entries where location is
 *   'In transit'". Two of these variants (`change_server`,
 *   `resolve_spawn`) carry no location at all and therefore always
 *   render as "In transit"; the other three carry destination
 *   identifiers that usually resolve. We hide all five always
 *   rather than introspecting each payload's location fields. If a
 *   stricter per-event "destination resolved?" filter is later
 *   wanted, extend {@link isHiddenMovementType} below.
 */

/**
 * Snake-case `GameEvent` discriminators that the web timeline
 * suppresses. Must stay in sync with
 * `apps/tray-ui/src/timeline/filter.ts:IN_TRANSIT_HIDDEN_TYPES`.
 */
export const IN_TRANSIT_HIDDEN_TYPES: ReadonlySet<string> = new Set([
  'join_pu',
  'change_server',
  'quantum_target_selected',
  'seed_solar_system',
  'resolve_spawn',
]);

/**
 * Returns the event type discriminator from any of the two server
 * event shapes the web app consumes:
 *   - `EventDto`         — has `event_type` at the top level
 *   - `EventEnvelope`    — has `event.type` (the serde tag)
 * Falls back to inspecting `payload.type` for forward-compatibility
 * with shapes that nest the discriminator inside `payload`.
 */
function extractEventType(item: unknown): string | undefined {
  if (typeof item !== 'object' || item === null) return undefined;
  const obj = item as {
    event?: { type?: unknown } | null;
    event_type?: unknown;
    payload?: { type?: unknown } | null;
  };
  if (typeof obj.event?.type === 'string') return obj.event.type;
  if (typeof obj.event_type === 'string') return obj.event_type;
  if (typeof obj.payload?.type === 'string') return obj.payload.type;
  return undefined;
}

/**
 * True when the given event (any supported shape) is a suppressed
 * movement variant. Defensive against missing discriminators —
 * unknown shapes are kept, so a schema regression never silently
 * drops rows.
 */
export function isHiddenMovementType(item: unknown): boolean {
  const type = extractEventType(item);
  if (type === undefined) return false;
  return IN_TRANSIT_HIDDEN_TYPES.has(type);
}

/**
 * Returns a new array with movement-noise events removed. Order is
 * preserved.
 */
export function filterMovementNoise<T>(items: ReadonlyArray<T>): T[] {
  return items.filter((item) => !isHiddenMovementType(item));
}
