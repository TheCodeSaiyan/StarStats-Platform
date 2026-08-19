import type { ReferenceCategory } from './reference-types';

export interface Quantiles {
  min: number;
  p10: number;
  p25: number;
  p50: number;
  p75: number;
  p90: number;
  max: number;
  n: number;
}

/** `peer_group -> metricPath -> Quantiles`. */
export type StatsGroups = Record<string, Record<string, Quantiles>>;

export interface CategoryStats {
  groups: StatsGroups;
}

export const ALL_BUCKET = '__all__';

const CATEGORY_PLURAL: Record<ReferenceCategory, string> = {
  vehicle: 'vehicles',
  weapon: 'weapons',
  item: 'items',
  location: 'locations',
};

const VEHICLE_FAMILY_LABEL: Record<string, string> = {
  combat: 'Combat ships',
  industrial: 'Industrial ships',
  transport: 'Transport ships',
  support: 'Support ships',
  ground: 'Ground vehicles',
  other: 'Other vehicles',
};

/** Human label for a peer-group bucket key within a category. */
export function compareLabel(category: ReferenceCategory, key: string): string {
  if (key === ALL_BUCKET) return `All ${CATEGORY_PLURAL[category]}`;
  if (category === 'vehicle' && VEHICLE_FAMILY_LABEL[key]) return VEHICLE_FAMILY_LABEL[key];
  // Other categories use slugified type/tier keys → de-slug + sentence case.
  const words = key.replace(/[-_]+/g, ' ').trim();
  return words ? words.charAt(0).toUpperCase() + words.slice(1) : key;
}

/**
 * Choose the metric→quantiles map for `peerGroup`, falling back to the
 * whole-category `__all__` bucket, then to an empty map. The viz layer
 * degrades to context-free values when a metric is absent from the
 * returned map, so an empty map is a safe (if plain) result.
 */
export function pickBucket(
  stats: CategoryStats,
  peerGroup: string,
): Record<string, Quantiles> {
  return stats.groups[peerGroup] ?? stats.groups[ALL_BUCKET] ?? {};
}
