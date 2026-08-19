/**
 * Parse raw class-name identifiers into hierarchy components for
 * the journey-page roll-ups.
 *
 * The wiki reference catalog is the authoritative source for
 * display names, but for *grouping* we need machine-extractable
 * dimensions — manufacturer, model family, size for items/weapons;
 * solar-system / body / place for locations. Both can be recovered
 * deterministically from the raw class_name string the engine emits.
 *
 *   `KLWE_LaserCannon_S2`        -> mfr=Klaus & Werner, family=Laser Cannon, size=S2
 *   `behr_rifle_ballistic_lh86`  -> mfr=Behring, family=Rifle Ballistic Lh86, size=null
 *   `OOC_Stanton_2b_Daymar`      -> system=Stanton, body=Daymar
 *   `OOC_Stanton_1_Hurston`      -> system=Stanton, body=Hurston
 *   `Orison_LOC`                 -> place=Orison (no system match → top-level place)
 *
 * No throws, never returns undefined for a non-empty input — callers
 * always get a usable shape to render.
 */

// ----- shared dictionaries ----------------------------------------

/** Lift the manufacturer-prefix dictionary from heuristic-name.ts so
 *  parsing stays consistent with display rendering. Keep in sync when
 *  new manufacturers show up in event data. */
const MANUFACTURER_NAMES: Record<string, string> = {
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
  AMRS: 'Amon & Reese',
  APAR: 'Apocalypse Arms',
  BEHR: 'Behring',
  GMNI: 'Gemini',
  HRST: 'Hurston Dynamics',
  KBAR: 'Kastak Arms',
  KLWE: 'Klaus & Werner',
  KSAR: 'Kastak Arms',
  PRAR: 'Preacher Armament',
  JOKR: 'Joker Engineering',
  MXOX: 'MaxOx',
};

/** Solar-system head tokens we recognise. The list stays short so a
 *  miss falls through to "Unknown" rather than mis-attributing a
 *  random first segment as a system name. Matched case-insensitively
 *  via `findSystem()` so `STANTON` / `stanton` / `Stanton` all
 *  resolve. The value is the canonical display form. */
const KNOWN_SYSTEMS: Record<string, string> = {
  stanton: 'Stanton',
  pyro: 'Pyro',
  nyx: 'Nyx',
  castra: 'Castra',
  terra: 'Terra',
  sol: 'Sol',
};

/** Map from a body token (case-insensitive lookup key) to its parent
 *  system. Used when the raw destination omits the system prefix —
 *  e.g. `Hurston_Lorville` should still land under Stanton. Value is
 *  the canonical body display form. */
const KNOWN_BODIES: Record<string, { system: string; display: string }> = {
  // -- Stanton planets --
  hurston: { system: 'Stanton', display: 'Hurston' },
  crusader: { system: 'Stanton', display: 'Crusader' },
  arccorp: { system: 'Stanton', display: 'ArcCorp' },
  microtech: { system: 'Stanton', display: 'microTech' },
  // -- Stanton Lagrange-point short codes (engine emits these as
  //    prefixes in destinations like `HUR_L1_Faithful_Dream`). They
  //    name the planet they orbit, so we map each to its full body. --
  hur: { system: 'Stanton', display: 'Hurston' },
  cru: { system: 'Stanton', display: 'Crusader' },
  arc: { system: 'Stanton', display: 'ArcCorp' },
  mic: { system: 'Stanton', display: 'microTech' },
  // -- Stanton moons (Hurston / Crusader / ArcCorp / microTech) --
  aberdeen: { system: 'Stanton', display: 'Aberdeen' },
  arial: { system: 'Stanton', display: 'Arial' },
  // Legacy quantum-target payloads use the misspelling `Ariel`.
  ariel: { system: 'Stanton', display: 'Arial' },
  magda: { system: 'Stanton', display: 'Magda' },
  ita: { system: 'Stanton', display: 'Ita' },
  cellin: { system: 'Stanton', display: 'Cellin' },
  daymar: { system: 'Stanton', display: 'Daymar' },
  yela: { system: 'Stanton', display: 'Yela' },
  wala: { system: 'Stanton', display: 'Wala' },
  lyria: { system: 'Stanton', display: 'Lyria' },
  calliope: { system: 'Stanton', display: 'Calliope' },
  clio: { system: 'Stanton', display: 'Clio' },
  euterpe: { system: 'Stanton', display: 'Euterpe' },
  // -- Pyro planets / dwarfs --
  bloom: { system: 'Pyro', display: 'Bloom' },
  monox: { system: 'Pyro', display: 'Monox' },
  terminus: { system: 'Pyro', display: 'Terminus' },
  // -- Pyro V moons --
  adir: { system: 'Pyro', display: 'Adir' },
  vatra: { system: 'Pyro', display: 'Vatra' },
  vuur: { system: 'Pyro', display: 'Vuur' },
  fairo: { system: 'Pyro', display: 'Fairo' },
  fuego: { system: 'Pyro', display: 'Fuego' },
  ignis: { system: 'Pyro', display: 'Ignis' },
  // -- Hurston Dynamics short code (engine emits `HurDyn_*` for
  //    several Hurston-surface installations). --
  hurdyn: { system: 'Stanton', display: 'Hurston' },
  // -- Rest Stops (engine `LOC_RR_S<n>_L<m>`) live at Stanton's
  //    Lagrange points but aren't tied to any single body. We
  //    file them under Stanton with a synthetic 'Rest Stops'
  //    body so the rollup still has a meaningful three-level
  //    hierarchy (Stanton -> Rest Stops -> S1 L3). --
  rr: { system: 'Stanton', display: 'Rest Stops' },
  // -- HDMS = Hurston Defense Material Storage. Engine emits
  //    `HDMS_<name>` for outposts spread across the four Hurston
  //    moons (Aberdeen/Magda/Ita/Arial). Without per-entry moon
  //    resolution we group them collectively under Stanton ->
  //    HDMS Outposts. Tier 0 catalog wins when the wiki catalogues
  //    individual HDMS outposts with parent metadata. --
  hdms: { system: 'Stanton', display: 'HDMS Outposts' },
  // -- Shubin Mining facilities (`Shubin_*`). Primarily on the
  //    microTech moons (Calliope / Clio) but a few elsewhere. Same
  //    "group collectively until catalogued" pattern as HDMS. --
  shubin: { system: 'Stanton', display: 'Shubin Outposts' },
  // -- System stars themselves (when the engine references the
  //    star, it does so via `<System>Star`). Treat as the system. --
  stantonstar: { system: 'Stanton', display: 'Stanton' },
  pyrostar: { system: 'Pyro', display: 'Pyro' },
  nyxstar: { system: 'Nyx', display: 'Nyx' },
  // -- Nyx bodies --
  delamar: { system: 'Nyx', display: 'Delamar' },
};

/** Map from a place token to its hierarchy. Covers the user-visible
 *  cities, stations, and outposts that show up in event payloads
 *  without a system/body prefix (e.g. `Orison_LOC`, `GrimHEX`). When
 *  the raw place spans multiple segments (`Port_Olisar`,
 *  `New_Babbage`), the joined form is also keyed so the lookup hits
 *  either way. */
const KNOWN_PLACES: Record<
  string,
  { system: string; body: string; display: string }
> = {
  // -- Stanton cities --
  lorville: { system: 'Stanton', body: 'Hurston', display: 'Lorville' },
  orison: { system: 'Stanton', body: 'Crusader', display: 'Orison' },
  area18: { system: 'Stanton', body: 'ArcCorp', display: 'Area18' },
  newbabbage: { system: 'Stanton', body: 'microTech', display: 'New Babbage' },
  babbage: { system: 'Stanton', body: 'microTech', display: 'New Babbage' },
  // -- Stanton stations --
  grimhex: { system: 'Stanton', body: 'Yela', display: 'GrimHEX' },
  portolisar: { system: 'Stanton', body: 'Crusader', display: 'Port Olisar' },
  olisar: { system: 'Stanton', body: 'Crusader', display: 'Port Olisar' },
  everusharbor: { system: 'Stanton', body: 'Hurston', display: 'Everus Harbor' },
  porttressler: { system: 'Stanton', body: 'microTech', display: 'Port Tressler' },
  baijinipoint: { system: 'Stanton', body: 'ArcCorp', display: 'Baijini Point' },
  seraphim: { system: 'Stanton', body: 'Crusader', display: 'Seraphim Station' },
  kareah: { system: 'Stanton', body: 'Crusader', display: 'Security Post Kareah' },
  // -- Pyro --
  ruinstation: { system: 'Pyro', body: 'Pyro V', display: 'Ruin Station' },
  endgame: { system: 'Pyro', body: 'Pyro V', display: 'Endgame' },
  checkmate: { system: 'Pyro', body: 'Pyro V', display: 'Checkmate' },
  rappel: { system: 'Pyro', body: 'Pyro V', display: 'Rappel' },
  starlight: { system: 'Pyro', body: 'Pyro V', display: 'Starlight Service Station' },
  // -- Nyx --
  levski: { system: 'Nyx', body: 'Delamar', display: 'Levski' },
  // -- Rayari research outposts (scattered across microTech moons).
  //    We can't tell which moon without seeing the suffix, so we
  //    attribute them to microTech generally. Drop the body level
  //    when a specific moon is known. --
  rayari: { system: 'Stanton', body: 'microTech', display: 'Rayari Outpost' },
};

/** Match S2 / Mk3 / V1 etc. — uppercased on render. */
const SIZE_PATTERNS: RegExp[] = [
  /^S\d+$/i,
  /^M\d+$/i,
  /^Mk\d+$/i,
  /^V\d+$/i,
  /^F\d+$/i,
];

/** Tokens that don't carry semantic weight in event data — strip
 *  before grouping. */
const SKIP_SEGMENTS = new Set([
  'NPC',
  'AI',
  'OOC',
  'PROC',
  'LandingArea',
  'LOC',
]);

// ----- weapon / item parsing --------------------------------------

export interface WeaponParts {
  manufacturer: string | null;
  family: string;
  /** S2 / Mk3 / etc. — null when the class has no recognised size tag. */
  size: string | null;
  /** Raw class for fall-through display + tooltips. */
  raw: string;
}

/** Parse a weapon-class identifier into manufacturer / family / size.
 *  Used to group Combat > Top weapons by mfr and to attach size badges
 *  on the family row. */
export function parseWeaponClass(raw: string): WeaponParts {
  const parts = stripAndSplit(raw);
  const { manufacturer, rest } = lookupManufacturer(parts);
  const { sizes, nonSize } = pickSizes(rest);
  const family =
    nonSize.length > 0
      ? titleCase(splitCamelCase(nonSize.join(' ')))
      : 'Unknown';
  return {
    manufacturer,
    family,
    size: sizes[0] ?? null,
    raw,
  };
}

export interface ItemParts {
  manufacturer: string | null;
  /** Single-row display label for the item — family + variant joined,
   *  size tag preserved. */
  model: string;
  raw: string;
}

/** Item classes have inconsistent shapes (`klwe_pistol_energy_*`,
 *  `rsi_odyssey_undersuit_*`, etc.), so we don't try to split
 *  family/variant — just lift the manufacturer and present everything
 *  else as one model string. The grouping dimension is manufacturer. */
export function parseItemClass(raw: string): ItemParts {
  const parts = stripAndSplit(raw);
  const { manufacturer, rest } = lookupManufacturer(parts);
  const model =
    rest.length > 0
      ? rest
          .map((p) =>
            isSize(p) ? p.toUpperCase() : titleCase(splitCamelCase(p)),
          )
          .join(' ')
      : 'Unknown';
  return { manufacturer, model, raw };
}

// ----- location parsing -------------------------------------------

export interface LocationParts {
  system: string | null;
  body: string | null;
  place: string | null;
  raw: string;
}

/** Subset of `LocationCatalog` the parser actually reads. Loose
 *  shape so we don't drag the full `reference.ts` interface into a
 *  pure parsing module — keeps this file free of network types. */
export interface LocationCatalogLookup {
  byName: ReadonlyMap<string, LocationLookupEntry>;
  byTag: ReadonlyMap<string, LocationLookupEntry>;
  bySlug: ReadonlyMap<string, LocationLookupEntry>;
}

/** Subset of `LocationEntry` the parser actually reads. */
export interface LocationLookupEntry {
  displayName: string;
  system: string | null;
  parent: string | null;
  classification: string | null;
}

/** Parse a location / destination / planet identifier into
 *  system / body / place. Best-effort — four tiers, in order:
 *    0. Catalog hit on any token (when `catalog` is provided)
 *    1. Leading-system match (`OOC_Stanton_2_Crusader`)
 *    2. Body match (`Hurston_Lorville` → infer Stanton)
 *    3. Place match (`Orison_LOC` → infer Stanton/Crusader/Orison)
 *  Only falls through to "no system" for truly unrecognised tokens.
 *
 *  Catalog-first invariant: the wiki catalog is the authoritative
 *  source for any name that exists there. The hardcoded dicts
 *  below (KNOWN_SYSTEMS / KNOWN_BODIES / KNOWN_PLACES) survive
 *  only because they handle engine-only short codes the wiki
 *  doesn't index (`HUR_L1`, `KSP_*`, `MIC_*` Lagrange / station
 *  prefixes). Adding a new wiki-indexed place here is a smell —
 *  prefer fixing the catalog enrichment instead. */
export function parseLocationClass(
  raw: string,
  catalog?: LocationCatalogLookup,
): LocationParts {
  const parts = stripAndSplit(raw);
  if (parts.length === 0) {
    return { system: null, body: null, place: null, raw };
  }

  // Synthetic top-level matchers — engine patterns that don't fit
  // the system/body/place hierarchy cleanly (cross-cutting objects
  // like Jump Points, Comm Arrays). Each matcher returns a result
  // with a *synthetic* top-level grouping ("Jump Points",
  // "Communications") so the rollup keeps a meaningful tree shape.
  // See docs/REFERENCE-CATALOG-HIERARCHY.md for the rationale.
  for (const m of SYNTHETIC_MATCHERS) {
    const hit = m(parts, raw);
    if (hit) return hit;
  }

  // Tier 0 — wiki catalog hit on any token. This is the authoritative
  // source; everything beneath is fallback for engine-only forms
  // that don't appear in the wiki. Try the joined form first (catches
  // `Stanton1b`-style tags), then each segment by name.
  if (catalog) {
    const tier0 = lookupCatalog(parts, raw, catalog);
    if (tier0) return tier0;
  }

  // Tier 1 — system token in the segments.
  const systemIdx = parts.findIndex((p) => KNOWN_SYSTEMS[p.toLowerCase()]);
  if (systemIdx !== -1) {
    const system = KNOWN_SYSTEMS[parts[systemIdx].toLowerCase()];
    const tail = parts.slice(systemIdx + 1).filter((p) => !isPlanetIndex(p));
    return resolveAfterSystem(system, tail, raw);
  }

  // Tier 2 — body token anywhere in the segments.
  const namedParts = parts.filter((p) => !isPlanetIndex(p));
  const bodyIdx = namedParts.findIndex((p) => KNOWN_BODIES[p.toLowerCase()]);
  if (bodyIdx !== -1) {
    const bodyMeta = KNOWN_BODIES[namedParts[bodyIdx].toLowerCase()];
    const after = namedParts.slice(bodyIdx + 1);
    const place =
      after.length > 0 ? resolvePlace(after) : null;
    return {
      system: bodyMeta.system,
      body: bodyMeta.display,
      place,
      raw,
    };
  }

  // Tier 3 — place token anywhere in the segments. Try the longest
  // joined form first so multi-word place names match before their
  // individual tokens (`Port_Olisar` should win over `Olisar`).
  const placeHit = findPlaceMatch(namedParts);
  if (placeHit) {
    return {
      system: placeHit.system,
      body: placeHit.body,
      place: placeHit.display,
      raw,
    };
  }

  // No tier matched — keep the title-cased whole as a place under
  // "Other / unmapped". Genuinely unrecognised destinations.
  return {
    system: null,
    body: null,
    place: titleCase(splitCamelCase(parts.join(' '))),
    raw,
  };
}

/** Tail after the system was matched — figure out body / place. */
function resolveAfterSystem(
  system: string,
  namedTail: string[],
  raw: string,
): LocationParts {
  if (namedTail.length === 0) {
    return { system, body: null, place: null, raw };
  }
  // Prefer a known-body match in the tail. Otherwise the first
  // segment is treated as the body (matches legacy behavior).
  const bodyIdx = namedTail.findIndex((p) => KNOWN_BODIES[p.toLowerCase()]);
  if (bodyIdx !== -1) {
    const bodyMeta = KNOWN_BODIES[namedTail[bodyIdx].toLowerCase()];
    const after = namedTail.slice(bodyIdx + 1);
    return {
      system,
      body: bodyMeta.display,
      place: after.length > 0 ? resolvePlace(after) : null,
      raw,
    };
  }
  if (namedTail.length === 1) {
    return {
      system,
      body: titleCase(splitCamelCase(namedTail[0])),
      place: null,
      raw,
    };
  }
  return {
    system,
    body: titleCase(splitCamelCase(namedTail[0])),
    place: titleCase(splitCamelCase(namedTail.slice(1).join(' '))),
    raw,
  };
}

/** Render a place from one or more trailing segments. Prefers a
 *  catalog hit (so `Olisar` becomes "Port Olisar") and falls back to
 *  title-cased camel-case splitting. */
function resolvePlace(parts: string[]): string {
  // Try the joined form first (`Port_Olisar` → `portolisar` key).
  const joined = parts.join('').toLowerCase();
  if (KNOWN_PLACES[joined]) {
    return KNOWN_PLACES[joined].display;
  }
  // Then any single segment (`Olisar` → `olisar` key).
  for (const p of parts) {
    const hit = KNOWN_PLACES[p.toLowerCase()];
    if (hit) return hit.display;
  }
  return titleCase(splitCamelCase(parts.join(' ')));
}

/** Walk segments looking for a place match (single or joined).
 *  Returns the full hierarchy when found. */
function findPlaceMatch(
  parts: string[],
):
  | { system: string; body: string; display: string }
  | null {
  // Try the full joined form first (catches `Port_Olisar`,
  // `New_Babbage` even when there's no system/body context).
  const joined = parts.join('').toLowerCase();
  if (KNOWN_PLACES[joined]) return KNOWN_PLACES[joined];
  // Then each individual segment.
  for (const p of parts) {
    const hit = KNOWN_PLACES[p.toLowerCase()];
    if (hit) return hit;
  }
  return null;
}

// ----- helpers ----------------------------------------------------

/** Drop runtime prefixes ([PROC], LandingArea_, OOC_, NPC_, AI_) and
 *  return the meaningful tokens. Exported so rollup helpers can
 *  re-tokenize unmapped raws to derive a grouping key.
 *
 *  Splits joined `<System><index>` tokens like `Stanton2_X` or
 *  `Pyro4a_Y` into `[System, index, ...]` so downstream tier
 *  lookups see the system as its own segment. The engine emits
 *  these joined forms inconsistently across destination payloads. */
export function stripAndSplit(raw: string): string[] {
  const trimmed = raw.trim().replace(/^\[[A-Z_]+\]/, '');
  const segments = trimmed
    .split('_')
    .map((p) => p.trim())
    .filter((p) => p.length > 0 && !SKIP_SEGMENTS.has(p));
  // Rewrite any `<System><digit>...` segment into separate tokens.
  // Keeps the order so later positional logic (system found at idx
  // N → tail starts at N+1) still works.
  const expanded: string[] = [];
  for (const s of segments) {
    const split = splitSystemIndex(s);
    expanded.push(...split);
  }
  return expanded;
}

/** Split a token like `Stanton2`, `Stanton4a`, or `Pyro5` into
 *  `[System, index]`. Returns the input unchanged when no match. */
function splitSystemIndex(segment: string): string[] {
  // Build the alternation once. KNOWN_SYSTEMS keys are lowercase so
  // we match case-insensitively; the canonical display is recovered
  // by the downstream system lookup.
  const match = segment.match(SYSTEM_INDEX_REGEX);
  if (!match) return [segment];
  return [match[1], match[2]];
}

const SYSTEM_INDEX_REGEX = new RegExp(
  `^(${Object.keys(KNOWN_SYSTEMS).join('|')})(\\d+[a-z]?)$`,
  'i',
);

/** Filter for destination buckets — returns true when the raw value
 *  is an engine-internal marker that shouldn't appear in a
 *  destinations list at all (mission objectives, nav points, object
 *  containers, party member markers). Callers drop these before
 *  rolling up so they never leak into the bucket bars. */
export function isNonDestination(raw: string): boolean {
  const v = raw.trim();
  return NON_DESTINATION_PATTERNS.some((re) => re.test(v));
}

/**
 * Best-effort friendly label for a raw travel-destination / place-visited
 * value, for the routes / places-visited / travel widgets (which otherwise
 * render the raw engine id — the "not resolving" reports). Handles the three
 * formats those endpoints actually emit:
 *   - mission quantum beacons (M9 dynamic ids, `MISSION_QT_Quantum_Beacon_
 *     718…`) → "Mission beacon" — per-mission, never catalogued;
 *   - pipe-delimited hierarchies (`Stanton|microTech|New Babbage`) → the
 *     most specific non-empty segment;
 *   - engine class ids (`LOC_RR_S1_L1`, `NewBabbage_LOC`) → via
 *     {@link parseLocationClass}, most-specific part first.
 * Falls back to the raw when nothing better is derivable.
 */
export function prettyLocationLabel(
  raw: string,
  catalog?: LocationCatalogLookup,
): string {
  const v = (raw ?? '').trim();
  if (!v || /^\|+$/.test(v)) return 'Unknown';
  // Mission quantum beacons — per-mission dynamic destinations.
  if (/^mission[_\s]/i.test(v) && /(quantum|beacon|_qt_|qt$)/i.test(v)) {
    return /salvage/i.test(v) ? 'Mission beacon · salvage' : 'Mission beacon';
  }

  // Fixed station aliases must run before the generic `rs_ext_*`
  // jump-point parser. The engine uses the same prefix for orbital
  // stations (`rs_ext_cru-leo1`) and real jump points
  // (`rs_ext_stan-pyro_jp1`); only the latter has a `jp<N>` suffix.
  const station = stationAliasLabel(v);
  if (station) return station;

  // Procedural destinations are real places a player can quantum to,
  // but their runtime ids / socpak UUIDs are not stable identities.
  // Mirror starstats-core's synthetic labels so repeated instances merge.
  const lower = v.toLowerCase();
  if (/^objectcontainer_reststop$/i.test(v)) return 'Rest Stop';
  if (lower.includes('ab_mine')) return 'Asteroid mining node';
  if (lower.includes('ab_collector')) return 'Gas collection node';
  if (lower.includes('_cluster_') || lower.endsWith('.socpak')) {
    return 'Asteroid cluster';
  }
  if (lower.includes('racing_static')) return 'Race track';
  // Pipe hierarchy: the last non-empty segment is the most specific place.
  if (v.includes('|')) {
    const segs = v.split('|').map((s) => s.trim()).filter(Boolean);
    if (segs.length === 0) return 'Unknown';
    const last = segs[segs.length - 1];
    const p = parseLocationClass(last, catalog);
    return p.place ?? p.body ?? p.system ?? last;
  }
  const p = parseLocationClass(v, catalog);
  if (p.system === 'Jump Points' && p.body && p.place) {
    return `${p.body} · ${p.place}`;
  }
  if (p.system === 'Communications') {
    const servedSystem = p.body && p.body !== 'Unknown system' ? p.body : null;
    return [servedSystem, 'Comm Array', p.place].filter(Boolean).join(' ');
  }
  if (p.body === 'Rest Stops' && p.place) return `Rest Stop ${p.place}`;
  return p.place ?? p.body ?? p.system ?? v;
}

/** Canonical Stanton station names for the engine-only aliases also
 * maintained by `starstats-core::location_classifier`. S1-S4 are body
 * sectors (HUR/CRU/ARC/MIC), not star-system numbers. */
const STANTON_STATION_NAMES: Readonly<Record<string, string>> = {
  'HUR:LEO': 'Everus Harbor',
  'CRU:LEO': 'Seraphim Station',
  'ARC:LEO': 'Baijini Point',
  'MIC:LEO': 'Port Tressler',
  'HUR:L1': 'HUR-L1 Green Glade Station',
  'HUR:L2': 'HUR-L2 Faithful Dream Station',
  'HUR:L3': 'HUR-L3 Thundering Express Station',
  'HUR:L4': 'HUR-L4 Melodic Fields Station',
  'HUR:L5': 'HUR-L5 High Course Station',
  'CRU:L1': 'CRU-L1 Ambitious Dream Station',
  'CRU:L4': 'CRU-L4 Shallow Fields Station',
  'CRU:L5': 'CRU-L5 Beautiful Glen Station',
  'ARC:L1': 'ARC-L1 Wide Forest Station',
  'ARC:L2': 'ARC-L2 Lively Pathway Station',
  'ARC:L3': 'ARC-L3 Modern Express Station',
  'ARC:L4': 'ARC-L4 Faint Glen Station',
  'ARC:L5': 'ARC-L5 Yellow Core Station',
  'MIC:L1': 'MIC-L1 Shallow Frontier Station',
  'MIC:L2': 'MIC-L2 Long Forest Station',
  'MIC:L3': 'MIC-L3 Endless Odyssey Station',
  'MIC:L4': 'MIC-L4 Red Crossroads Station',
  'MIC:L5': 'MIC-L5 Modern Icarus Station',
};

function stationAliasLabel(raw: string): string | null {
  const parts = stripAndSplit(raw);
  let sector: string | null = null;
  let slot: string | null = null;

  if (parts.length === 3 && parts[0].toLowerCase() === 'rr') {
    [, sector, slot] = parts;
  } else if (
    parts.length === 3 &&
    parts[0].toLowerCase() === 'rs' &&
    /^(ext|entry|comm)$/i.test(parts[1])
  ) {
    [sector, slot] = parts[2].split('-', 2);
  }
  if (!sector || !slot) return null;

  const normalizedSector =
    ({ S1: 'HUR', S2: 'CRU', S3: 'ARC', S4: 'MIC' } as const)[
      sector.toUpperCase() as 'S1' | 'S2' | 'S3' | 'S4'
    ] ?? sector.toUpperCase();
  const normalizedSlot = /^LEO1?$/i.test(slot) ? 'LEO' : slot.toUpperCase();
  return STANTON_STATION_NAMES[`${normalizedSector}:${normalizedSlot}`] ?? null;
}

/**
 * Relabel a set of raw destination/place buckets to friendly labels
 * ({@link prettyLocationLabel}) and MERGE buckets that collapse to the same
 * label (e.g. every per-mission beacon → one "Mission beacon" row), summing
 * counts and keeping the raws (for a single-entity KB link). Sorted by count
 * desc. This is what turns a list of raw engine ids into readable, deduped
 * top-routes / places-visited rows.
 */
export function aggregateLocationBuckets(
  buckets: ReadonlyArray<{ value: string; count: number }>,
  catalog?: LocationCatalogLookup,
): Array<{ label: string; count: number; raws: string[] }> {
  const byLabel = new Map<string, { count: number; raws: string[] }>();
  for (const b of buckets) {
    // A moving party member and a hand-placed nav marker are valid
    // quantum targets, but not stable destinations. Their runtime ids
    // would otherwise fragment the leaderboard into one row per session.
    if (/^(PartyMemberMarker|NavPoint_Dynamic)(_|$)/i.test(b.value.trim())) {
      continue;
    }
    const label = prettyLocationLabel(b.value, catalog);
    const e = byLabel.get(label) ?? { count: 0, raws: [] };
    e.count += b.count;
    e.raws.push(b.value);
    byLabel.set(label, e);
  }
  return [...byLabel.entries()]
    .map(([label, e]) => ({ label, count: e.count, raws: e.raws }))
    .sort((a, b) => b.count - a.count);
}

const NON_DESTINATION_PATTERNS: RegExp[] = [
  // Mission objective markers — `Mission_Marker_*`, `Mission_*`.
  /^Mission(_|$)/i,
  // Engine container references — `ObjectContainer*`.
  /^ObjectContainer/i,
  // HUD nav-point markers placed manually by the pilot.
  /^NavPoint/i,
  // Party member / friend markers.
  /^PartyMember/i,
];

/** Registry of synthetic-pseudo-system matchers. Each function takes
 *  the tokenized segments and the original raw string and returns a
 *  `LocationParts` when it recognises the engine pattern, or `null`
 *  to defer to the next matcher / the standard tier chain.
 *
 *  Adding a new pattern is one entry here — no surgery in the parser
 *  body needed. Document the rationale for each top-level pseudo-
 *  system in `docs/REFERENCE-CATALOG-HIERARCHY.md`. */
type SyntheticMatcher = (parts: string[], raw: string) => LocationParts | null;

const SYNTHETIC_MATCHERS: SyntheticMatcher[] = [
  matchJumpPoint,
  matchCommArray,
];

/** Match an engine `rs_ext_<a>-<b>_jp<N>` jump-point identifier and
 *  build a LocationParts using the `Jump Points` pseudo-system. The
 *  shape from `stripAndSplit` is `['rs', 'ext', '<a>-<b>', 'jp<N>']`
 *  (LOC_ already stripped, casing varies). Returns null when the
 *  segments don't fit the JP pattern. */
function matchJumpPoint(parts: string[], raw: string): LocationParts | null {
  if (parts.length < 3) return null;
  const head = parts[0]?.toLowerCase();
  const kind = parts[1]?.toLowerCase();
  if (head !== 'rs' || kind !== 'ext') return null;
  const routeRaw = parts[2];
  const jpToken = parts[3] ?? null;
  return {
    system: 'Jump Points',
    body: prettifyRoute(routeRaw),
    place: jpToken ? `Jump Point ${jpToken.replace(/^jp/i, '').toUpperCase()}` : null,
    raw,
  };
}

/** Render an `<a>-<b>` route token as `"A ↔ B"`, mapping known
 *  short codes to canonical system names. Unknown codes get
 *  title-cased so the row stays readable. */
function prettifyRoute(route: string): string {
  const parts = route.split('-').map(prettifyRouteToken);
  return parts.join(' ↔ ');
}

function prettifyRouteToken(token: string): string {
  const known: Record<string, string> = {
    stan: 'Stanton',
    stanton: 'Stanton',
    pyro: 'Pyro',
    nyx: 'Nyx',
    castra: 'Castra',
    terra: 'Terra',
    sol: 'Sol',
    magnus: 'Magnus',
    cru: 'Crusader',
  };
  return known[token.toLowerCase()] ?? titleCase(splitCamelCase(token));
}

/** Match engine Comm-Array identifiers and route them to a
 *  synthetic `Communications` pseudo-system. The engine names these
 *  in a couple of shapes:
 *    `Comm_Array_Lagrange_Stanton_L1_HUR-L1`
 *    `CommArray_<id>`
 *    `<system>_Comm_Array_<id>`
 *  We match whenever the joined segments contain the literal
 *  `commarray` token (case-insensitive). When we can spot a known
 *  system token in the segments we propagate it as the parent body
 *  so the comm array is shown next to the system it serves. */
function matchCommArray(parts: string[], raw: string): LocationParts | null {
  const joined = parts.join('').toLowerCase();
  const hasComm =
    joined.includes('commarray') ||
    parts.some((p, i) => {
      const lower = p.toLowerCase();
      return (
        lower === 'comm' &&
        parts[i + 1]?.toLowerCase() === 'array'
      );
    });
  if (!hasComm) return null;
  // Try to detect which system the comm array serves. Pick the
  // first segment that resolves as a system or system short-code.
  let serves: string | null = null;
  for (const p of parts) {
    const lower = p.toLowerCase();
    if (KNOWN_SYSTEMS[lower]) {
      serves = KNOWN_SYSTEMS[lower];
      break;
    }
    if (KNOWN_BODIES[lower]) {
      serves = KNOWN_BODIES[lower].system;
      break;
    }
  }
  // The trailing identifier (`HUR-L1`, station id, etc.) goes in
  // the place slot. Take everything that isn't `comm`, `array`, or
  // a known system/body token.
  const tail = parts.filter((p) => {
    const lower = p.toLowerCase();
    if (lower === 'comm' || lower === 'array' || lower === 'commarray') return false;
    if (KNOWN_SYSTEMS[lower]) return false;
    if (KNOWN_BODIES[lower]) return false;
    return true;
  });
  const placeLabel =
    tail.length > 0 ? titleCase(splitCamelCase(tail.join(' '))) : null;
  return {
    system: 'Communications',
    body: serves ?? 'Unknown system',
    place: placeLabel,
    raw,
  };
}

/** Catalog Tier 0 lookup. Walks token candidates against the three
 *  catalog indexes (tag, name, slug) in priority order. First hit
 *  wins. Returns null when nothing matches so the parser can drop
 *  through to the dictionary tiers. */
function lookupCatalog(
  parts: string[],
  raw: string,
  catalog: LocationCatalogLookup,
): LocationParts | null {
  // Try a few candidate keys in order of confidence:
  //   1. joined-no-underscore form of all parts ("Stanton1b")
  //   2. joined-no-underscore form of the first two parts
  //   3. each individual part on its own
  // Each candidate is matched against tag/name/slug indexes.
  const candidates: string[] = [];
  if (parts.length >= 2) {
    candidates.push((parts[0] + parts[1]).toLowerCase());
  }
  candidates.push(parts.join('').toLowerCase());
  for (const p of parts) candidates.push(p.toLowerCase());

  for (const key of candidates) {
    const hit =
      catalog.byTag.get(key) ??
      catalog.byName.get(key) ??
      catalog.bySlug.get(key);
    if (!hit) continue;
    // A `Star`-classification hit is just the system itself —
    // upgrade to (system, null, null) rather than putting the star
    // as a "place".
    if (hit.classification === 'Star') {
      return {
        system: hit.system ?? hit.displayName,
        body: null,
        place: null,
        raw,
      };
    }
    // Planets are the body level. parent is null (it's the system).
    if (hit.classification === 'Planet') {
      return {
        system: hit.system,
        body: hit.displayName,
        place: null,
        raw,
      };
    }
    // Moon / city / station / outpost / etc — full hierarchy.
    return {
      system: hit.system,
      body: hit.parent,
      place: hit.displayName,
      raw,
    };
  }
  return null;
}

function lookupManufacturer(parts: string[]): {
  manufacturer: string | null;
  rest: string[];
} {
  if (parts.length === 0) return { manufacturer: null, rest: parts };
  const headUpper = parts[0].toUpperCase();
  const mfr = MANUFACTURER_NAMES[headUpper] ?? null;
  return {
    manufacturer: mfr,
    rest: mfr ? parts.slice(1) : parts,
  };
}

function isSize(s: string): boolean {
  return SIZE_PATTERNS.some((re) => re.test(s));
}

/** Separate size tags from the rest — sizes get pulled to a separate
 *  list so the family name stays clean ("Laser Cannon" not "Laser
 *  Cannon S2"). */
function pickSizes(parts: string[]): {
  sizes: string[];
  nonSize: string[];
} {
  const sizes: string[] = [];
  const nonSize: string[] = [];
  for (const p of parts) {
    if (isSize(p)) sizes.push(p.toUpperCase());
    else nonSize.push(p);
  }
  return { sizes, nonSize };
}

/** Planet-index tokens are lowercase digit-letter combos like '1',
 *  '2b', '4a'. They sit between the system and the body and don't
 *  carry useful display info. */
function isPlanetIndex(s: string): boolean {
  return /^\d+[a-z]?$/i.test(s);
}

function splitCamelCase(s: string): string {
  return s.replace(/([a-z])([A-Z])/g, '$1 $2');
}

function titleCase(s: string): string {
  return s
    .split(/\s+/)
    .map((w) =>
      w.length === 0 ? w : w[0].toUpperCase() + w.slice(1).toLowerCase(),
    )
    .join(' ');
}
