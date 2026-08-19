/**
 * Loadout mapping helpers for the paperdoll and carried-gear views.
 *
 * Provides:
 * - LoadoutItem                       — per-item payload shape from tray burst
 * - prettify                          — class-name → human-readable fallback
 * - EXCLUDED_PORTS / isExcludedPort  — anatomy & HUD ports filtered from display
 * - BodySlot / slotForClassification — FPS.Armor.* → paperdoll slot
 * - GearGroup / groupForItem         — per-item carried-gear bucket
 */

// ---------------------------------------------------------------------------
// Per-item payload type — emitted by the tray in BurstSummary loadout payloads
// ---------------------------------------------------------------------------

/** A single equipped item as emitted by the tray's loadout-restore BurstSummary. */
export interface LoadoutItem {
  class: string;
  port: string;
  category: string;
}

// ---------------------------------------------------------------------------
// Prettify — class-name fallback display helper
// ---------------------------------------------------------------------------

/**
 * Strips a trailing underscore-plus-digits suffix then title-cases each
 * underscore-separated word. Preserves inner capitalisation.
 *
 * Examples:
 *   "rsi_p4ar_01"      → "Rsi P4ar"
 *   "GRIN_Light_Helmet" → "GRIN Light Helmet"
 */
export function prettify(className: string): string {
  const stripped = className.replace(/_\d+$/, '');
  return stripped
    .split('_')
    .map((word) => word.charAt(0).toUpperCase() + word.slice(1))
    .join(' ');
}

// ---------------------------------------------------------------------------
// Port exclusion — anatomy cosmetics + internal HUD attachment points
// ---------------------------------------------------------------------------

/**
 * Lowercased set of item-port names that should never appear in the paperdoll
 * or gear list. Two categories:
 *  - Anatomy cosmetics (hair, eyes, beard …) — character appearance only
 *  - HUD / internal mounts (radar, mobiglas …) — non-gear technical ports
 */
export const EXCLUDED_PORTS: ReadonlySet<string> = new Set([
  // Anatomy cosmetics
  'body_itemport',
  'beard_itemport',
  'eyebrow_itemport',
  'eyedetail_itemport',
  'eyelashes_itemport',
  'eyes_itemport',
  'hair_itemport',
  'head_itemport',
  'stubble_itemport',
  'teeth_itemport',
  'universal_scalp_itemport',
  'universal_necksock',
  'universal_necksock_undersuit',
  // HUD / internal mounts
  'radar',
  'mobiglas_attach',
  'mobiglas_screen_attach',
  'legacy_mobiglas_screen_attach',
  'lens_itemport',
]);

/** Returns true when the port should be hidden from the paperdoll / gear list. Case-insensitive. */
export function isExcludedPort(port: string): boolean {
  return EXCLUDED_PORTS.has(port.toLowerCase());
}

// ---------------------------------------------------------------------------
// Body slot mapping — FPS.Armor.* classification → paperdoll slot
// ---------------------------------------------------------------------------

export type BodySlot = 'head' | 'torso' | 'arms' | 'legs' | 'undersuit' | 'back';

const CLASSIFICATION_TO_SLOT: ReadonlyMap<string, BodySlot> = new Map([
  ['FPS.Armor.Helmet', 'head'],
  ['FPS.Armor.Torso', 'torso'],
  ['FPS.Armor.Arms', 'arms'],
  ['FPS.Armor.Legs', 'legs'],
  ['FPS.Armor.Undersuit', 'undersuit'],
  ['FPS.Armor.Backpack', 'back'],
]);

/**
 * Maps an item classification string to a paperdoll body slot.
 * Returns null for non-armor classifications (weapons, consumables, etc.)
 * and for undefined/unknown values.
 */
export function slotForClassification(c?: string): BodySlot | null {
  if (c === undefined) return null;
  return CLASSIFICATION_TO_SLOT.get(c) ?? null;
}

// ---------------------------------------------------------------------------
// Gear group mapping — classification + port → carried-gear bucket
// ---------------------------------------------------------------------------

export type GearGroup =
  | 'weapons'
  | 'magazines'
  | 'attachments'
  | 'throwables'
  | 'utility'
  | 'consumables'
  | 'other';

/**
 * Determines the carried-gear group for an item.
 *
 * Priority order:
 * 1. Classification prefix: FPS.Weapon.* → weapons, FPS.WeaponAttachment.Magazine → magazines,
 *    other FPS.WeaponAttachment.* → attachments, FPS.Consumable.* → consumables
 * 2. Port name pattern: grenade_attach* → throwables, utility_attach* / module_attach → utility
 * 3. Final fallback → other
 */
export function groupForItem(
  classification: string | undefined,
  port: string,
  _fallbackCategory?: string,
): GearGroup {
  if (classification !== undefined) {
    if (classification.startsWith('FPS.Weapon.')) return 'weapons';
    if (classification === 'FPS.WeaponAttachment.Magazine') return 'magazines';
    if (classification.startsWith('FPS.WeaponAttachment.')) return 'attachments';
    if (classification.startsWith('FPS.Consumable.')) return 'consumables';
  }

  const lowerPort = port.toLowerCase();
  if (lowerPort.startsWith('grenade_attach')) return 'throwables';
  if (lowerPort.startsWith('utility_attach') || lowerPort === 'module_attach') return 'utility';

  return 'other';
}

// ---------------------------------------------------------------------------
// Burst selection — pick the loadout snapshot to display
// ---------------------------------------------------------------------------

/** The loadout-restore burst payload shape (kind + items). */
export interface LoadoutBurstPayload {
  kind: 'loadout_restore';
  items: LoadoutItem[];
}

/** Type guard for a loadout_restore burst payload. */
export function isLoadoutBurstPayload(p: unknown): p is LoadoutBurstPayload {
  if (typeof p !== 'object' || p === null) return false;
  const obj = p as Record<string, unknown>;
  return obj['kind'] === 'loadout_restore' && Array.isArray(obj['items']);
}

/** Minimal event shape needed to find a loadout burst. */
interface BurstEventLike {
  event_type?: string;
  payload?: unknown;
}

/**
 * Picks the loadout snapshot to display from a list of events.
 *
 * A burst is emitted for ANY run of 3+ attachment events, so a partial
 * re-equip produces a small burst while a full spawn produces a large
 * one. The most RECENT burst is therefore often partial. We instead pick
 * the burst with the MOST items — the user's fullest captured loadout —
 * so the paperdoll reflects a complete snapshot rather than a recent
 * fragment. Ties resolve to the first (newest, since events are
 * newest-first).
 *
 * Returns the chosen event, or undefined when no loadout burst exists.
 */
export function pickFullestLoadoutBurst<T extends BurstEventLike>(
  events: readonly T[],
): T | undefined {
  let best: T | undefined;
  let bestCount = -1;
  for (const e of events) {
    if (e.event_type !== 'burst_summary') continue;
    if (!isLoadoutBurstPayload(e.payload)) continue;
    const n = e.payload.items.length;
    if (n > bestCount) {
      best = e;
      bestCount = n;
    }
  }
  return best;
}
