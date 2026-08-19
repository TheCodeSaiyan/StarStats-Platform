/**
 * Timezone-safe conversions between a `<input type="datetime-local">` value and
 * a UTC ISO instant.
 *
 * `datetime-local` has no UTC mode: it always emits/consumes a NAIVE wall-clock
 * string (`YYYY-MM-DDTHH:MM`) interpreted in the user's local zone. The only
 * correct bridge is the browser's `getTimezoneOffset()` — which returns
 * `UTC − local` in minutes, so a zone AHEAD of UTC is NEGATIVE (UTC+2 → -120)
 * and a zone BEHIND is positive (UTC-5 → +300).
 *
 * Both helpers are RUNTIME-TIMEZONE-INDEPENDENT: they never call
 * `new Date(naiveString)` (which would parse in the *runtime's* zone — wrong in
 * a server action). Instead the wall-clock numbers are parsed as if UTC, then
 * shifted by the caller-supplied offset. This makes the pair a symmetric fixed
 * point regardless of where the code runs.
 */

const MINUTE_MS = 60_000;

/** `YYYY-MM-DDTHH:MM` (16 chars) — the shape a datetime-local input emits. */
const LOCAL_INPUT_LEN = 16;

/**
 * Convert a naive datetime-local value (wall-clock in the user's zone) plus that
 * zone's `getTimezoneOffset()` into a UTC ISO instant. Returns `null` when the
 * value is blank or unparseable, so callers can treat "no expiry" uniformly.
 */
export function localInputToUtcIso(
  localValue: string,
  offsetMinutes: number,
): string | null {
  const v = localValue.trim();
  if (v === '') return null;
  // Parse the wall-clock numbers as UTC to get a baseline independent of the
  // runtime's zone. A datetime-local value has no seconds, so append them.
  const asUtcMs = Date.parse(v.length === LOCAL_INPUT_LEN ? `${v}:00Z` : `${v}Z`);
  if (Number.isNaN(asUtcMs)) return null;
  // local = UTC − offset  ⇒  UTC = local + offset. `asUtcMs` holds the local
  // wall-clock numbers, so the true instant is asUtcMs + offset.
  return new Date(asUtcMs + offsetMinutes * MINUTE_MS).toISOString();
}

/**
 * Inverse of {@link localInputToUtcIso}: a UTC ISO instant → the naive
 * datetime-local value (`YYYY-MM-DDTHH:MM`) for the given zone. Returns an empty
 * string for an unparseable ISO so it can feed a form `defaultValue` directly.
 */
export function utcIsoToLocalInput(iso: string, offsetMinutes: number): string {
  const ms = Date.parse(iso);
  if (Number.isNaN(ms)) return '';
  // Undo the shift, then read back the wall-clock numbers as the local value.
  return new Date(ms - offsetMinutes * MINUTE_MS)
    .toISOString()
    .slice(0, LOCAL_INPUT_LEN);
}
