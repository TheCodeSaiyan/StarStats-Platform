/**
 * Boundary parser + view helpers for RSI Ship Matrix enrichment.
 *
 * The detail endpoint returns `metadata` as an opaque
 * `Record<string, unknown>` (the backend owns the regenerated schema
 * types; the web side must NOT assume their shape). The Ship Matrix
 * enrichment plugin writes its blob under `metadata.ship_matrix` with
 * the contract fixed in the design spec:
 *
 *   {
 *     specs: { length, beam, height, mass, scm_speed,
 *              afterburner_speed, min_crew, max_crew, cargo },
 *     production_status: string,
 *     description: string,
 *     media: string[],
 *     matched_by: "name" | "chassis",
 *     matched_at: string
 *   }
 *
 * Everything here is PURE (no `server-only`, no fetch) so it can be
 * unit-tested and imported by the server component that renders the
 * vehicle KB page. We validate defensively at the boundary: any field
 * that isn't the expected primitive is dropped rather than coerced, so
 * a malformed upstream blob degrades to a partial render instead of a
 * crash. The whole thing returns `null` when `ship_matrix` is absent
 * or isn't an object — the caller then renders nothing.
 */

/** Numeric Ship Matrix specs. Every field is optional — the upstream
 *  source frequently omits values, and the parser drops anything that
 *  isn't a finite number. */
export interface ShipMatrixSpecs {
  length?: number;
  beam?: number;
  height?: number;
  mass?: number;
  scm_speed?: number;
  afterburner_speed?: number;
  min_crew?: number;
  max_crew?: number;
  cargo?: number;
}

/** Validated Ship Matrix blob. `media` is always an array (possibly
 *  empty) so gallery callers don't have to null-check. */
export interface ShipMatrix {
  specs: ShipMatrixSpecs;
  production_status?: string;
  description?: string;
  media: string[];
  matched_by?: string;
  matched_at?: string;
}

function isRecord(v: unknown): v is Record<string, unknown> {
  return typeof v === 'object' && v !== null && !Array.isArray(v);
}

function asFiniteNumber(v: unknown): number | undefined {
  return typeof v === 'number' && Number.isFinite(v) ? v : undefined;
}

function asNonEmptyString(v: unknown): string | undefined {
  return typeof v === 'string' && v.length > 0 ? v : undefined;
}

const SPEC_KEYS: ReadonlyArray<keyof ShipMatrixSpecs> = [
  'length',
  'beam',
  'height',
  'mass',
  'scm_speed',
  'afterburner_speed',
  'min_crew',
  'max_crew',
  'cargo',
];

function parseSpecs(raw: unknown): ShipMatrixSpecs {
  if (!isRecord(raw)) return {};
  const out: ShipMatrixSpecs = {};
  for (const key of SPEC_KEYS) {
    const n = asFiniteNumber(raw[key]);
    if (n !== undefined) out[key] = n;
  }
  return out;
}

function parseMedia(raw: unknown): string[] {
  if (!Array.isArray(raw)) return [];
  return raw.filter(
    (v): v is string => typeof v === 'string' && v.length > 0,
  );
}

/**
 * Validate an opaque `metadata.ship_matrix` value into a typed
 * {@link ShipMatrix}, or `null` when it's absent / not an object.
 * Pass `metadata.ship_matrix` (i.e. already indexed off the opaque
 * metadata record) — this function does not reach into `metadata`
 * itself.
 */
export function parseShipMatrix(raw: unknown): ShipMatrix | null {
  if (!isRecord(raw)) return null;
  return {
    specs: parseSpecs(raw.specs),
    production_status: asNonEmptyString(raw.production_status),
    description: asNonEmptyString(raw.description),
    media: parseMedia(raw.media),
    matched_by: asNonEmptyString(raw.matched_by),
    matched_at: asNonEmptyString(raw.matched_at),
  };
}

/** Pull `metadata.ship_matrix` off an opaque metadata record and parse
 *  it. Convenience wrapper so the page doesn't index the opaque blob
 *  inline. */
export function shipMatrixFromMetadata(
  metadata: Record<string, unknown>,
): ShipMatrix | null {
  return parseShipMatrix(metadata.ship_matrix);
}

/**
 * Gate the Ship Matrix section: it only renders for the `vehicle`
 * category AND when `metadata.ship_matrix` validates. Returns the
 * parsed blob (truthy) to render, or `null` to skip. Centralises the
 * "vehicle-only" rule so the page and its tests agree.
 */
export function shipMatrixForCategory(
  category: string,
  metadata: Record<string, unknown>,
): ShipMatrix | null {
  if (category !== 'vehicle') return null;
  return shipMatrixFromMetadata(metadata);
}

export interface SpecRow {
  label: string;
  value: string;
}

// Dimension specs and their display units, in render order.
const DIMENSION_ROWS: ReadonlyArray<{
  key: keyof ShipMatrixSpecs;
  label: string;
  unit: string;
}> = [
  { key: 'length', label: 'Length', unit: 'm' },
  { key: 'beam', label: 'Beam', unit: 'm' },
  { key: 'height', label: 'Height', unit: 'm' },
  { key: 'mass', label: 'Mass', unit: 'kg' },
];

const SPEED_ROWS: ReadonlyArray<{
  key: keyof ShipMatrixSpecs;
  label: string;
  unit: string;
}> = [
  { key: 'scm_speed', label: 'SCM speed', unit: 'm/s' },
  { key: 'afterburner_speed', label: 'Afterburner', unit: 'm/s' },
];

/** Format a number with thousands separators (no locale surprises in
 *  jsdom — `toLocaleString` is stable enough for this). */
function fmtNum(n: number): string {
  return n.toLocaleString('en-US');
}

/**
 * Build the labelled spec rows for the specs grid, gracefully omitting
 * any field the parser dropped. Order: dimensions → speeds → crew →
 * cargo → production status. Crew is collapsed into a single range row
 * (`min–max`, or a single value when they're equal).
 */
export function shipMatrixSpecRows(sm: ShipMatrix): SpecRow[] {
  const rows: SpecRow[] = [];
  const { specs } = sm;

  for (const { key, label, unit } of DIMENSION_ROWS) {
    const n = specs[key];
    if (n !== undefined) rows.push({ label, value: `${fmtNum(n)} ${unit}` });
  }

  for (const { key, label, unit } of SPEED_ROWS) {
    const n = specs[key];
    if (n !== undefined) rows.push({ label, value: `${fmtNum(n)} ${unit}` });
  }

  const crew = crewValue(specs.min_crew, specs.max_crew);
  if (crew !== undefined) rows.push({ label: 'Crew', value: crew });

  if (specs.cargo !== undefined) {
    rows.push({ label: 'Cargo', value: `${fmtNum(specs.cargo)} SCU` });
  }

  if (sm.production_status) {
    rows.push({ label: 'Production status', value: sm.production_status });
  }

  return rows;
}

/**
 * Build the gallery image URLs — one per `media[]` index.
 *
 * These land in `<img src>` and are fetched by the BROWSER, so they MUST
 * be same-origin RELATIVE paths. The previous version embedded
 * `apiBase()` — the server-side `STARSTATS_API_URL` (an internal compose
 * hostname like `http://starstats-api:8080`) — which the client can't
 * resolve, producing a DNS error and a broken image. They now point at
 * the web's own `/kb/media/[category]/[className]/[idx]` route handler,
 * which proxies to the API server-side. Indexed by position so the real
 * RSI URL never reaches the client; the backend serves it dark via the
 * `STARSTATS_SHIP_MATRIX_MEDIA` kill-switch (404), which the gallery
 * degrades on. Empty array when the blob has no media.
 */
export function shipMatrixMediaUrls(sm: ShipMatrix, className: string): string[] {
  const cls = encodeURIComponent(className);
  return sm.media.map((_url, idx) => `/kb/media/vehicle/${cls}/${idx}`);
}

function crewValue(
  min: number | undefined,
  max: number | undefined,
): string | undefined {
  if (min === undefined && max === undefined) return undefined;
  if (min !== undefined && max !== undefined) {
    return min === max ? `${min}` : `${min}–${max}`;
  }
  // Only one bound present — show whichever we have.
  return `${(min ?? max) as number}`;
}
