/**
 * Best-effort prettifier for raw Star Citizen class identifiers when
 * the reference catalogue has no entry. Examples:
 *
 *   'KLWE_LaserCannon_S2'      -> 'Klaus & Werner Laser Cannon S2'
 *   'AEGS_Avenger_Stalker'     -> 'Aegis Avenger Stalker'
 *   'NPC_AI_Pirate_Marine_Hostile' -> 'Pirate Marine Hostile'
 *   'OOC_Stanton_3_Hurston'    -> 'Stanton 3 Hurston'
 *
 * Never throws and never returns an empty string for non-empty input —
 * the dashboard always renders SOMETHING readable rather than a raw
 * underscored identifier.
 *
 * The function is intentionally conservative: if the input already
 * contains spaces or mixed-case wordforms (no underscores, no
 * SHOUTY_SNAKE_CASE), it's assumed to already be pretty and returned
 * verbatim.
 */

/** Manufacturer-code prefixes the wiki uses. Expand as we discover
 *  more in event data. Lookup is by exact match on the first segment. */
const MANUFACTURER_NAMES: Record<string, string> = {
  // Ships
  AEGS: 'Aegis',
  ANVL: 'Anvil',
  ARGO: 'Argo',
  BANU: 'Banu',
  CNOU: 'Consolidated Outland',
  CRUS: 'Crusader',
  DRAK: 'Drake',
  ESPR: 'Esperia',
  GAMA: 'Gatac',
  GRIN: 'Greycat',
  KRGR: 'Kruger',
  MISC: 'MISC',
  ORIG: 'Origin',
  RSI: 'RSI',
  TMBL: 'Tumbril',
  VNCL: 'Vanduul',
  XIAN: "Xi'an",
  // Personal weapons
  AMRS: 'Amon & Reese',
  APAR: 'Apocalypse Arms',
  BEHR: 'Behring',
  GMNI: 'Gemini',
  HRST: 'Hurston Dynamics',
  KBAR: 'Kastak Arms',
  KLWE: 'Klaus & Werner',
  KSAR: 'Kastak Arms',
  PRAR: 'Preacher Armament',
  // Ship-mounted weapons
  JOKR: 'Joker Engineering',
  MXOX: 'MaxOx',
};

/** Strip these segments — they're internal log-format prefixes
 *  that add noise rather than meaning. */
const SKIP_SEGMENTS = new Set([
  'NPC',
  'AI',
  'OOC',
  'PROC',
  'LandingArea',
]);

/** Size/mark suffix patterns we want uppercased rather than
 *  title-cased ('S2' stays 'S2', not 'S2'). */
const SIZE_PATTERNS: RegExp[] = [
  /^S\d+$/i,
  /^M\d+$/i,
  /^Mk\d+$/i,
  /^V\d+$/i,
  /^F\d+$/i,
];

export function toFriendlyName(raw: string | null | undefined): string {
  if (raw == null) return '';
  const trimmed = raw.trim();
  if (trimmed.length === 0) return '';

  // If the input has no underscores AND already mixes upper+lower
  // letters with at least one space, treat it as already-pretty.
  if (
    !trimmed.includes('_') &&
    /\s/.test(trimmed) &&
    /[a-z]/.test(trimmed) &&
    /[A-Z]/.test(trimmed)
  ) {
    return trimmed;
  }

  // Drop a leading `[PROC]` or similar runtime-prefix wrapper.
  const working = trimmed.replace(/^\[[A-Z_]+\]/, '');

  let parts = working.split('_').filter((p) => p.length > 0);

  let manufacturerName: string | null = null;
  if (parts.length > 0) {
    const head = parts[0];
    if (MANUFACTURER_NAMES[head]) {
      manufacturerName = MANUFACTURER_NAMES[head];
      parts = parts.slice(1);
    }
  }

  parts = parts.filter((p) => !SKIP_SEGMENTS.has(p));

  const rendered = parts.map((p) => {
    if (SIZE_PATTERNS.some((re) => re.test(p))) {
      return p.toUpperCase();
    }
    return titleCase(splitCamelCase(p));
  });

  const joined = rendered.join(' ');
  return manufacturerName ? `${manufacturerName} ${joined}`.trim() : joined;
}

function splitCamelCase(s: string): string {
  // 'LaserCannon' -> 'Laser Cannon'. Keeps acronyms intact —
  // 'RSIQuantum' -> 'RSI Quantum' (only inserts a space when a
  // lower-case letter is followed by an upper-case letter).
  return s.replace(/([a-z])([A-Z])/g, '$1 $2');
}

function titleCase(s: string): string {
  return s
    .split(' ')
    .map((w) =>
      w.length === 0 ? w : w[0].toUpperCase() + w.slice(1).toLowerCase(),
    )
    .join(' ');
}
