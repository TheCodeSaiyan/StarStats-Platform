import 'server-only';
import { cookies } from 'next/headers';
import { LENSES, type Lens } from '@/lib/lens';

/**
 * Which lens `/me` opens on.
 *
 * Overview (no lens) is the projection's landing state by design — the ring,
 * the callouts and the trace. But a reader who works in the lists had to
 * re-open their lens on every visit, and the review flagged that the first
 * thing they see has no lists in it.
 *
 * So the choice is remembered. First visit still lands on overview; after that
 * you land where you left.
 *
 * A COOKIE, read server-side, for the same reason the calibration is: the lens
 * decides what the first paint contains, so resolving it in the browser would
 * show overview and then swap. `localStorage` cannot be read during SSR and
 * would guarantee that flash.
 *
 * Per-device, unlike the tile layout, which is on the account. Syncing it would
 * mean a new `UserPreferences` field — that schema is a fixed set (`kb_view` is
 * the closest precedent), so it is a server change with a migration rather
 * than a client one. Worth doing if readers ask for it; not assumed here.
 *
 * The stored value is the lens ID, never the rail INDEX: the rail's order is a
 * presentation decision and has already changed once, which would silently
 * reinterpret a saved index as a different lens.
 */
export const LENS_COOKIE = 'ss-lens';

/** ~1 year, matching the calibration cookie: "set once" should stick. */
export const LENS_COOKIE_MAX_AGE = 60 * 60 * 24 * 365;

/** The sentinel for "overview", which is not a lens. */
export const OVERVIEW = 'overview';

function isLensId(v: string | undefined): v is Lens {
  return !!v && LENSES.some((l) => l.id === v);
}

/**
 * The saved lens as a rail index, or -1 for overview.
 *
 * Anything unrecognised — a stale ID from a removed lens, a hand-edited
 * cookie — falls back to overview rather than throwing or guessing.
 */
export async function getInitialLensIndex(): Promise<number> {
  const store = await cookies();
  const raw = store.get(LENS_COOKIE)?.value;
  if (!isLensId(raw)) return -1;
  const i = LENSES.findIndex((l) => l.id === raw);
  return i;
}
