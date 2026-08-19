/**
 * Single source of truth for reference-data attribution.
 *
 * SINCE THE M10 CUTOVER the shipped reference catalogue redistributes
 * FACTS ONLY — engine class names, display names, slugs, per-category
 * factual specs and taxonomy — plus CIG/RSI first-party data. It does
 * NOT redistribute the community wiki's copyrightable DESCRIPTION prose
 * anywhere (the generator drops every free-text field at extraction
 * time; see `scripts/generate.mjs`). Facts are not copyrightable, and
 * the names/specs themselves originate with Cloud Imperium.
 *
 * Attribution is therefore CIG/RSI ONLY. The former Star Citizen Wiki /
 * CC BY-SA 4.0 credit has been REMOVED from this module — keeping it
 * while we no longer carry wiki prose would misattribute facts and
 * assert a licence obligation that no longer applies. The CIG Ship
 * Matrix disclaimer is retained: vehicle spec sheets / imagery are
 * © Cloud Imperium and shown as unofficial fan reference.
 *
 * The exact Ship Matrix disclaimer string here is byte-identical to
 * what shipped before — do NOT paraphrase; `ShipMatrixDisclaimer.test.tsx`
 * / `ShipMatrixSection.test.tsx` assert the wording verbatim.
 */

import type { DataProvenance } from './types';

/**
 * Provenance of the reference catalogue as SHIPPED. `'rsi-cig'` since
 * the M10 cutover — facts + CIG-derived data, no wiki prose. This flag
 * governs which credits render.
 */
export const DATA_PROVENANCE: DataProvenance = 'rsi-cig';

/** One upstream data source + how to credit it. */
export interface AttributionSource {
  /** Stable id used by the snapshot manifest's `source_ids`. */
  id: string;
  /** Display name (`Cloud Imperium / Roberts Space Industries`). */
  name: string;
  /** Primary link shown next to the name. */
  url: string;
  /** Which categories this source contributes to. */
  appliesTo: readonly ('vehicle' | 'weapon' | 'item' | 'location')[];
  /** Whether this is community-contributed or a first-party rights holder. */
  kind: 'community' | 'first-party';
}

/**
 * Cloud Imperium / Roberts Space Industries — the first-party rights
 * holder for Star Citizen's names, specifications, and taxonomy. The
 * facts StarStats redistributes originate here.
 */
export const RSI_CIG: AttributionSource = {
  id: 'rsi-cig',
  name: 'Cloud Imperium / Roberts Space Industries',
  url: 'https://robertsspaceindustries.com',
  appliesTo: ['vehicle', 'weapon', 'item', 'location'],
  kind: 'first-party',
};

/** RSI's official Ship Matrix source (first-party, CIG-owned). */
export const RSI_SHIP_MATRIX: AttributionSource = {
  id: 'rsi-ship-matrix',
  name: 'Ship Matrix',
  url: 'https://robertsspaceindustries.com/ship-matrix',
  appliesTo: ['vehicle'],
  kind: 'first-party',
};

/**
 * All sources backing the shipped catalogue, in credit order. CIG/RSI
 * only since the M10 cutover.
 */
export const SOURCES: readonly AttributionSource[] = [RSI_CIG, RSI_SHIP_MATRIX];

/**
 * The Ship Matrix / CIG disclaimer, verbatim.
 *
 * MUST stay byte-identical to the text asserted by
 * `apps/web/src/components/kb/ShipMatrixDisclaimer.test.tsx` and
 * rendered by `ShipMatrixDisclaimer.tsx`.
 */
export const SHIP_MATRIX_DISCLAIMER =
  'Ship specifications, descriptions and images © Cloud Imperium Rights ' +
  'LLC / Cloud Imperium Rights Ltd. StarStats is an unofficial fan site, ' +
  'not endorsed by or affiliated with Cloud Imperium Group.';

/**
 * Structured facts for the CIG/RSI attribution. Surfaces that weave the
 * credit through links (about page, KB footers) read the name/URL from
 * here; the prose that links them stays in the surface.
 */
export const CIG_ATTRIBUTION = {
  sourceName: RSI_CIG.name,
  sourceUrl: RSI_CIG.url,
} as const;
