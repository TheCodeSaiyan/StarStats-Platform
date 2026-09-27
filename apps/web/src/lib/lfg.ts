/**
 * Looking for Group: labels for the server's closed vocabularies, and the
 * one piece of arithmetic the board shows (time left on a post).
 *
 * The vocabularies themselves come from `GET /v1/lfg/options`, so a value
 * the server adds shows up in the form without a web release; a value this
 * file has no label for falls back to a readable form of its id.
 */

const ACTIVITY: Record<string, string> = {
  bounty_hunting: 'Bounty hunting',
  mercenary: 'Mercenary',
  fps: 'FPS',
  mining: 'Mining',
  salvage: 'Salvage',
  hauling: 'Hauling',
  exploration: 'Exploration',
  racing: 'Racing',
  medical: 'Medical',
  piracy: 'Piracy',
  social: 'Social',
  other: 'Other',
};

const REGION: Record<string, string> = {
  any: 'Any region',
  eu: 'Europe',
  na: 'North America',
  sa: 'South America',
  oce: 'Oceania',
  asia: 'Asia',
};

const VOICE: Record<string, string> = {
  none: 'No voice',
  optional: 'Voice optional',
  required: 'Voice required',
};

function fallback(id: string): string {
  const s = id.replace(/_/g, ' ');
  return s.charAt(0).toUpperCase() + s.slice(1);
}

export const activityLabel = (id: string): string => ACTIVITY[id] ?? fallback(id);
export const regionLabel = (id: string): string => REGION[id] ?? fallback(id);
export const voiceLabel = (id: string): string => VOICE[id] ?? fallback(id);

/**
 * "1 h 20 min left", "5 min left", or "ending" in the last minute. The board
 * is rendered on the server, so this is as of the request.
 */
export function timeLeft(expiresAt: string, now: number): string {
  const mins = Math.floor((Date.parse(expiresAt) - now) / 60_000);
  if (mins < 1) return 'ending';
  const h = Math.floor(mins / 60);
  const m = mins % 60;
  if (h === 0) return `${m} min left`;
  return m === 0 ? `${h} h left` : `${h} h ${m} min left`;
}
