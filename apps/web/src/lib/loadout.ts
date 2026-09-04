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

/**
 * Item-port names that identify a paperdoll slot on their own.
 *
 * The port comes from the game's own `<AttachmentReceived>` line and says
 * where the item is WORN, so it holds for any armour the player owns.
 * `classification` comes from the reference catalogue, which only holds
 * what has been scraped — a new or missing set resolves to nothing, and
 * every piece of it then falls through to the carried-gear "Other" bucket
 * while the paperdoll renders empty. Observed with the CDS Combat
 * Superheavy set: suit, arms, legs, helmet and backpack all present in the
 * payload with correct ports, none of them in the catalogue.
 */
const PORT_TO_SLOT: ReadonlyMap<string, BodySlot> = new Map([
  ['armor_helmet', 'head'],
  ['armor_torso', 'torso'],
  ['armor_core', 'torso'],
  ['armor_arms', 'arms'],
  ['armor_legs', 'legs'],
  ['armor_undersuit', 'undersuit'],
  ['armor_backpack', 'back'],
  ['backpack', 'back'],
]);

/**
 * Maps an item PORT to a paperdoll slot. Case-insensitive.
 *
 * The fallback for `slotForClassification` — see `PORT_TO_SLOT`. Returns
 * null for any port that is not an armour mount.
 */
export function slotForPort(port: string): BodySlot | null {
  return PORT_TO_SLOT.get(port.toLowerCase()) ?? null;
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
  event_timestamp?: string | null;
}

/**
 * Ports that only a FULL restore fills.
 *
 * The tray emits a burst for any run of 3+ attachments, so swapping a
 * weapon looks structurally identical to respawning in a fresh kit —
 * both are "a burst". What separates them is the body: the engine
 * re-attaches the character body and its undersuit on a spawn restore
 * and never touches them for a re-equip. Either port present means the
 * snapshot describes a whole character rather than a fragment of one.
 */
const RESTORE_ANCHOR_PORTS: ReadonlySet<string> = new Set([
  'body_itemport',
  'armor_undersuit',
]);

/** True when the burst looks like a full spawn restore, not a partial re-equip. */
export function isCompleteRestore(payload: LoadoutBurstPayload): boolean {
  return payload.items.some((item) =>
    RESTORE_ANCHOR_PORTS.has(item.port.toLowerCase()),
  );
}

/** One selectable loadout snapshot, for the history picker. */
export interface LoadoutSnapshot<T> {
  event: T;
  /** RFC3339 capture time, or '' when the event carries none. */
  timestamp: string;
  /** Items before display filtering — the size the burst was recorded at. */
  itemCount: number;
  /** Whether this is a full restore (see `isCompleteRestore`). */
  complete: boolean;
}

/**
 * Every loadout snapshot in `events`, newest first.
 *
 * Feeds the history picker, which is the honest answer to a page that
 * can only ever show one snapshot: rather than guessing which one the
 * reader wants, show the chosen one and let them reach the rest.
 */
export function listLoadoutBursts<T extends BurstEventLike>(
  events: readonly T[],
): LoadoutSnapshot<T>[] {
  const out: LoadoutSnapshot<T>[] = [];
  for (const event of events) {
    if (event.event_type !== 'burst_summary') continue;
    if (!isLoadoutBurstPayload(event.payload)) continue;
    out.push({
      event,
      timestamp: event.event_timestamp ?? '',
      itemCount: event.payload.items.length,
      complete: isCompleteRestore(event.payload),
    });
  }
  return out.sort((a, b) => b.timestamp.localeCompare(a.timestamp));
}

/**
 * Picks the loadout snapshot to display.
 *
 * Order: an explicitly requested `selected` timestamp wins; otherwise the
 * most RECENT complete restore; otherwise the fullest burst of any kind.
 *
 * The old rule was "most items wins", full stop. It was aimed at a real
 * problem — a 3-item weapon swap should not replace a whole kit — but it
 * has no upper bound in time, so the largest burst ever recorded pins the
 * page permanently and no later kit can displace it. Observed in the
 * wild: a heavy-weapons snapshot kept showing for weeks while newer,
 * smaller full restores sat in the same response, and because the page
 * showed no date it read as current. Anchoring on "complete" instead of
 * "big" separates a spawn restore from a re-equip without freezing time.
 *
 * Returns undefined when no loadout burst exists.
 */
export function pickLoadoutBurst<T extends BurstEventLike>(
  events: readonly T[],
  selected?: string,
): T | undefined {
  const snapshots = listLoadoutBursts(events);
  if (snapshots.length === 0) return undefined;

  if (selected !== undefined && selected !== '') {
    const match = snapshots.find((s) => s.timestamp === selected);
    if (match) return match.event;
  }

  const latestComplete = snapshots.find((s) => s.complete);
  if (latestComplete) return latestComplete.event;

  return snapshots.reduce((best, s) =>
    s.itemCount > best.itemCount ? s : best,
  ).event;
}

