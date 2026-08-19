/**
 * Typed feature-flag registry for the metrics-redesign rollout.
 *
 * Per the impl plan §2.4, every metrics surface mounts behind one of
 * these flags. The `<MetricCard>` shell requires a `flagKey` prop so
 * TypeScript rejects any new card that forgets to wire one.
 */

export type MetricFlagKey =
  | 'metrics.year_heatmap'
  | 'metrics.sparkline_pills'
  | 'metrics.type_breakdown'
  | 'metrics.session_ribbon'
  | 'metrics.death_recap'
  | 'metrics.shard_stability'
  | 'metrics.activity_rhythm'
  | 'metrics.death_zones'
  | 'metrics.signature_move'
  | 'metrics.recap_card'
  | 'metrics.recap_compute'
  | 'metrics.now_strip';

const DEFAULTS: Record<MetricFlagKey, boolean> = {
  'metrics.year_heatmap': true,
  'metrics.sparkline_pills': true,
  'metrics.type_breakdown': true,
  'metrics.session_ribbon': true,
  'metrics.death_recap': true,
  'metrics.shard_stability': true,
  'metrics.activity_rhythm': true,
  'metrics.death_zones': true,
  'metrics.signature_move': true,
  'metrics.recap_card': true,
  'metrics.recap_compute': true,
  'metrics.now_strip': false,
};

export function isFlagEnabled(key: MetricFlagKey): boolean {
  return DEFAULTS[key];
}
