/**
 * `reference-data` — a build-time static package that consolidates the
 * StarStats reference catalogue (vehicle / weapon / item / location)
 * from committed JSON snapshots, plus the single source of truth for
 * reference-data attribution.
 *
 * ADDITIVE: this does NOT replace the runtime reference path in
 * `apps/web/src/lib/reference.ts` (which fetches `/v1/reference/*`
 * live). It is a new, separately-consumable static export.
 *
 * Attribution is exposed on a subpath (`reference-data/attribution`)
 * as well as here, so surfaces that only need the credit strings can
 * import them without pulling in the JSON snapshots.
 */

export * from './types';
export * from './attribution';
export * from './loader';
