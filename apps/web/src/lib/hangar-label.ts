/**
 * Turn a raw RSI pledge string into a friendly, linkable hangar label.
 *
 * The tray scrapes the RSI pledges page verbatim, so `HangarShip.name`
 * arrives as a category-prefixed string like:
 *   - "Standalone Ships - Railen"
 *   - "Paints - Railen - Uamchuai Paint"
 *   - "Subscribers Store - Salvaged Skull Relax to the Max Set"
 *   - "Upgrades - 300i to 325a"
 * which reads like a raw log line in the widget. `prettyHangarItem`
 * strips the leading RSI category segment(s) down to the item's own
 * name AND maps the pledge `kind` to an `<EntityLink>` category so the
 * widget can deep-link ships/weapons to the KB while leaving cosmetics
 * (paints, flair, upgrades) as plain text.
 *
 * Pure — no I/O. The catalogue lookup itself happens in the widget via
 * `<EntityLink>`; this only decides the display label + which category
 * (if any) the item belongs to.
 */

import type { ReferenceCategory } from './reference-types';

/** Category an item can deep-link to, or `null` when it's cosmetic /
 *  non-catalogued (paints, flair, upgrades, packages). */
export type HangarCategory = 'vehicle' | 'weapon' | null;

export interface PrettyHangarItem {
  /** Item name with the RSI category prefix(es) stripped. */
  label: string;
  /** `<EntityLink>` category, or `null` for cosmetic / non-linkable. */
  category: HangarCategory;
}

/**
 * Leading RSI category words we strip from a pledge name. The value is
 * the category hint used when the pledge `kind` is absent — a
 * `'vehicle'` / `'weapon'` word implies the item type; `null` marks a
 * cosmetic bucket (paints, flair, upgrades …) that must never link even
 * if the trailing name happens to look like a weapon.
 *
 * `undefined` is NOT a valid value here — membership is what matters;
 * the mapped value is the *type hint*. Keys are lowercased.
 */
const CATEGORY_PREFIXES: Record<string, HangarCategory> = {
  'standalone ships': 'vehicle',
  ships: 'vehicle',
  ship: 'vehicle',
  'ground vehicles': 'vehicle',
  vehicles: 'vehicle',
  vehicle: 'vehicle',
  weapons: 'weapon',
  weapon: 'weapon',
  // Cosmetic / non-catalogued buckets — explicitly no link.
  paints: null,
  paint: null,
  skins: null,
  skin: null,
  'subscribers store': null,
  upgrades: null,
  upgrade: null,
  'add-ons': null,
  'add-on': null,
  addons: null,
  flair: null,
  armor: null,
  armour: null,
  components: null,
  component: null,
  bundles: null,
  bundle: null,
  packages: null,
  package: null,
  'game packages': null,
  warbond: null,
  posters: null,
  poster: null,
  decorations: null,
  decoration: null,
};

/** Map a pledge `kind` to a link category. Returns `undefined` when the
 *  kind carries no usable signal (absent / unrecognised) so the caller
 *  falls through to the prefix hint, then a name heuristic. Returns an
 *  explicit `null` for known cosmetic kinds so they never link. */
function categoryFromKind(kind?: string | null): HangarCategory | undefined {
  if (!kind) return undefined;
  const k = kind.toLowerCase();
  if (/ship|vehicle/.test(k)) return 'vehicle';
  if (/weapon|rifle|gun|pistol|cannon/.test(k)) return 'weapon';
  if (/skin|paint|flair|decorat|poster|upgrade|add[-\s]?on|armou?r|component|package|bundle|consumable/.test(k)) {
    return null;
  }
  return undefined;
}

/** Last-resort heuristic: the trailing name obviously reads as a weapon. */
const WEAPON_NAME_RE = /\b(rifle|pistol|smg|shotgun|sniper|cannon|launcher|carbine|revolver|railgun)\b/i;

function weaponFromName(label: string): HangarCategory {
  return WEAPON_NAME_RE.test(label) ? 'weapon' : null;
}

/**
 * Strip RSI category prefixes from a pledge name and classify it.
 *
 * @param name  The raw `HangarShip.name` (e.g. "Paints - Railen - Foo").
 * @param kind  The pledge `kind` (e.g. "ship", "skin", "weapon"), when
 *              the API carries one.
 */
export function prettyHangarItem(
  name: string,
  kind?: string | null,
): PrettyHangarItem {
  const raw = (name ?? '').trim();
  const segments = raw.split(' - ').map((s) => s.trim()).filter((s) => s.length > 0);

  // Strip leading segments that are known RSI category words. Keep the
  // FIRST such word's type hint (e.g. "Paints" → cosmetic) even as we
  // peel further context segments (the ship a paint is for).
  let prefixCategory: HangarCategory | undefined;
  while (segments.length > 1) {
    const first = segments[0].toLowerCase();
    if (!(first in CATEGORY_PREFIXES)) break;
    if (prefixCategory === undefined) prefixCategory = CATEGORY_PREFIXES[first];
    segments.shift();
  }

  // The item's own name is the last remaining segment: for a paint
  // ("Railen - Uamchuai Paint" after stripping "Paints") that's the
  // paint name; for a ship ("Railen") it's the only segment.
  const label = segments.length > 0 ? segments[segments.length - 1] : raw;

  // Precedence: explicit kind → prefix hint → name heuristic. A cosmetic
  // signal from kind/prefix (explicit `null`) wins and does NOT fall
  // through to the weapon-name heuristic.
  const kindCategory = categoryFromKind(kind);
  let category: HangarCategory;
  if (kindCategory !== undefined) {
    category = kindCategory;
  } else if (prefixCategory !== undefined) {
    category = prefixCategory;
  } else {
    category = weaponFromName(label);
  }

  return { label, category };
}

/**
 * Known Star Citizen ship/vehicle manufacturer first-words. A bundle's
 * constituent items arrive as clean names (e.g. "Aegis Avenger Titan"),
 * with no RSI category prefix to lean on, so the ship heuristic keys off
 * the leading manufacturer token. Lowercased for case-insensitive match.
 */
const SHIP_MANUFACTURERS: ReadonlySet<string> = new Set([
  'aegis',
  'anvil',
  'origin',
  'drake',
  'misc',
  'crusader',
  'rsi',
  'consolidated',
  'aopoa',
  'banu',
  'esperia',
  'tumbril',
  'greycat',
  'argo',
  'kruger',
  'mirai',
  'gatac',
  'vanduul',
  'roberts',
]);

/**
 * Classify a bundle's constituent item name into a KB `<EntityLink>`
 * category. Unlike `prettyHangarItem` (which handles prefixed pledge
 * strings), a "Contains:" entry is already a clean item name, so we
 * decide purely from its shape:
 *
 *   1. an obvious weapon word → `'weapon'`
 *   2. a leading known ship manufacturer → `'vehicle'`
 *   3. otherwise → `'item'`
 *
 * We always return a real category (never `null`): contained items are
 * always wrapped in `<EntityLink>`, which degrades to plain text on a
 * catalog miss, so a wrong guess never produces a dead link.
 */
export function classifyContainedItem(name: string): ReferenceCategory {
  const clean = (name ?? '').trim();
  if (WEAPON_NAME_RE.test(clean)) return 'weapon';
  const first = clean.split(/\s+/)[0]?.toLowerCase() ?? '';
  if (SHIP_MANUFACTURERS.has(first)) return 'vehicle';
  return 'item';
}
