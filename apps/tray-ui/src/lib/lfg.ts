/**
 * Looking for Group helpers for the tray's Crew pane. Labels mirror
 * `apps/web/src/lib/lfg.ts`; the vocabularies themselves come from the
 * server's `/v1/lfg/options`, so an unknown value falls back to a readable
 * form of its id.
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
 * The server's spelling of a system the game log suggested, or `''` when
 * it is not one the board accepts. The log's classifier and the server
 * share one list, but a mismatch must pre-fill nothing rather than a value
 * the server would refuse.
 */
export function matchSystem(systems: readonly string[], suggestion: string | null): string {
  if (!suggestion) return '';
  return systems.find((s) => s.toLowerCase() === suggestion.trim().toLowerCase()) ?? '';
}

/** Every accepted crew member's handle, one per line, for pasting in game. */
export function crewHandles(members: ReadonlyArray<{ handle: string; status: string }>): string {
  return members
    .filter((m) => m.status === 'accepted')
    .map((m) => m.handle)
    .join('\n');
}
