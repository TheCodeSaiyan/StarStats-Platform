/**
 * Per-variant pretty rendering of a `GameEvent` JSON payload.
 *
 * Mirrors `format_summary` in `crates/starstats-client/src/commands.rs`
 * — keep the two in sync. The duplication is intentional: server-side
 * rendering on the web stays self-contained without an extra fetch
 * back to a Rust formatter, and the cost is one matched function.
 *
 * The payload shape comes from `#[serde(tag = "type", rename_all =
 * "snake_case")]` on `GameEvent` in `starstats-core`, so each variant
 * is a flat object with a `type` discriminator.
 *
 * Class-name resolution (P6 of the reference-registry rollout):
 *   - Each raw class identifier in the payload (weapon, item_class,
 *     vehicle_class, planet, destination, location_id, etc.) is
 *     resolved through the appropriate category Map in
 *     `ReferenceLookup`.
 *   - Catalog miss → falls through to `toFriendlyName()` so the UI
 *     never renders a raw underscored identifier.
 *   - Optional second arg accepts either the legacy `ReadonlyMap`
 *     (lowercased class → display) for vehicles, OR the new
 *     `ReferenceLookup` bundle covering all four categories. P7
 *     migrates callers to the bundle.
 */

import {
  EMPTY_REFERENCE_LOOKUP,
  prettyClass,
  type ReferenceLookup,
} from './reference';
import { toFriendlyName } from './heuristic-name';

interface BaseEvent {
  type: string;
  timestamp: string;
}

type GameEventPayload =
  | (BaseEvent & { type: 'process_init' })
  | (BaseEvent & { type: 'legacy_login'; handle: string })
  | (BaseEvent & {
      type: 'join_pu';
      address: string;
      port: number;
      shard: string;
      location_id: string;
    })
  | (BaseEvent & { type: 'change_server'; phase: 'start' | 'end' })
  | (BaseEvent & {
      type: 'seed_solar_system';
      solar_system: string;
      shard: string;
    })
  | (BaseEvent & {
      type: 'resolve_spawn';
      player_geid: string;
      fallback: boolean;
    })
  | (BaseEvent & {
      type: 'actor_death';
      victim: string;
      killer: string;
      weapon: string;
      damage_type: string;
    })
  | (BaseEvent & {
      type: 'vehicle_destruction';
      vehicle_class: string;
      destroy_level: number;
      caused_by: string;
    })
  | (BaseEvent & { type: 'hud_notification'; text: string })
  | (BaseEvent & {
      type: 'location_inventory_requested';
      player: string;
      location: string;
    })
  | (BaseEvent & { type: 'planet_terrain_load'; planet: string })
  | (BaseEvent & {
      type: 'quantum_target_selected';
      phase: 'fuel_requested' | 'selected';
      vehicle_class: string;
      destination: string;
    })
  | (BaseEvent & {
      type: 'attachment_received';
      item_class: string;
      port: string;
    })
  | (BaseEvent & {
      type: 'vehicle_stowed';
      vehicle_id: string;
      landing_area: string;
    })
  | (BaseEvent & {
      type: 'burst_summary';
      rule_id: string;
      size: number;
      end_timestamp: string;
      anchor_body_sample?: string | null;
    })
  | (BaseEvent & {
      type: 'player_death';
      body_class: string;
      body_id: string;
      zone?: string | null;
    })
  | (BaseEvent & {
      type: 'player_incapacitated';
      queue_id: number;
      zone?: string | null;
    })
  | (BaseEvent & {
      type: 'game_crash';
      channel: string;
      crash_dir_name: string;
      primary_log_name?: string | null;
      total_size_bytes: number;
    })
  | (BaseEvent & {
      type: 'launcher_activity';
      level: string;
      message: string;
      category: 'auth' | 'install' | 'patch' | 'update' | 'error' | 'info';
    })
  | (BaseEvent & {
      type: 'mission_start';
      mission_id: string;
      marker_kind: 'phase' | 'objective';
      mission_name?: string | null;
    })
  | (BaseEvent & {
      type: 'mission_end';
      mission_id?: string | null;
      outcome?: string | null;
    })
  | (BaseEvent & {
      type: 'shop_buy_request';
      shop_id?: string | null;
      item_class?: string | null;
      quantity?: number | null;
      raw: string;
    })
  | (BaseEvent & {
      type: 'shop_flow_response';
      shop_id?: string | null;
      success?: boolean | null;
      raw: string;
    })
  | (BaseEvent & {
      type: 'commodity_buy_request';
      commodity?: string | null;
      quantity?: number | null;
      raw: string;
    })
  | (BaseEvent & {
      type: 'commodity_sell_request';
      commodity?: string | null;
      quantity?: number | null;
      raw: string;
    })
  | (BaseEvent & {
      type: 'session_end';
      kind: 'system_quit' | 'fast_shutdown';
    })
  | (BaseEvent & {
      type: 'remote_match';
      rule_id: string;
      event_name: string;
      fields: Record<string, string>;
    });

/** Either the legacy vehicles-only Map or the full ReferenceLookup. */
export type ReferenceLookupArg =
  | ReferenceLookup
  | ReadonlyMap<string, string>
  | undefined;

/**
 * Format a payload into a one-liner summary. Falls back to a generic
 * "{type} event" if the payload doesn't match any known variant — that
 * way new server-side variants don't crash the dashboard, they just
 * render bare until the formatter learns them.
 */
export function formatEventSummary(
  payload: unknown,
  references?: ReferenceLookupArg,
): string {
  const lookup = asLookup(references);
  if (!isGameEventPayload(payload)) {
    if (
      typeof payload === 'object' &&
      payload !== null &&
      'type' in payload &&
      typeof (payload as { type: unknown }).type === 'string'
    ) {
      return `${(payload as { type: string }).type} event`;
    }
    return 'unknown event';
  }
  return formatKnown(payload, lookup);
}

/** Normalise the optional second arg into a full ReferenceLookup. The
 *  legacy `ReadonlyMap` shape (vehicles-only) is widened by placing
 *  it on the `vehicles` slot; other categories degrade to empty maps
 *  and therefore fall through to the heuristic fallback. */
function asLookup(arg: ReferenceLookupArg): ReferenceLookup {
  if (!arg) return EMPTY_REFERENCE_LOOKUP;
  // `'vehicles' in arg` narrows correctly: ReferenceLookup has this
  // property, a Map does not. Using `instanceof Map` doesn't work
  // here because `ReadonlyMap<K, V>` is an interface, not a class,
  // and TypeScript can't narrow against it via instanceof.
  if ('vehicles' in arg) return arg;
  return { ...EMPTY_REFERENCE_LOOKUP, vehicles: arg };
}

function formatKnown(
  event: GameEventPayload,
  lookup: ReferenceLookup,
): string {
  switch (event.type) {
    case 'process_init':
      return 'Game process started';
    case 'legacy_login':
      return `Logged in as ${event.handle}`;
    case 'join_pu': {
      const where = prettyClass(event.location_id, lookup.locations);
      return `Joined PU shard ${event.shard}${where ? ` · ${where}` : ''} (${event.address}:${event.port})`;
    }
    case 'change_server':
      return `Server transition: ${event.phase === 'start' ? 'starting' : 'complete'}`;
    case 'seed_solar_system':
      return `Seeded ${event.solar_system} on shard ${event.shard}`;
    case 'resolve_spawn':
      return `Spawn resolved (player ${event.player_geid}, fallback=${event.fallback})`;
    case 'actor_death': {
      // `killer` may be a player handle (already pretty) or an NPC
      // archetype like `NPC_AI_Pirate_Marine`. No catalog covers
      // NPCs yet — the heuristic strips the NPC_/AI_ prefixes and
      // title-cases the remainder.
      const killer = toFriendlyName(event.killer);
      const weapon = prettyClass(event.weapon, lookup.weapons);
      return `${event.victim} killed by ${killer} (${weapon}, ${event.damage_type})`;
    }
    case 'vehicle_destruction':
      return `Vehicle destroyed: ${prettyClass(event.vehicle_class, lookup.vehicles)} (level ${event.destroy_level}, by ${event.caused_by})`;
    case 'hud_notification':
      return `HUD: ${event.text.replace(/:\s*$/, '').replace(/:$/, '')}`;
    case 'location_inventory_requested':
      if (event.location === 'INVALID_LOCATION_ID') {
        return `${event.player} opened inventory (no location bound yet)`;
      }
      return `${event.player} opened inventory at ${prettyClass(event.location, lookup.locations)}`;
    case 'planet_terrain_load': {
      // Prefer the catalog; fall back to the heuristic (which strips
      // OOC_ prefixes etc.) rather than the original split-on-last-_
      // shortcut, since the heuristic is more thorough.
      const label = prettyClass(event.planet, lookup.locations);
      return `Near planet/moon: ${label || event.planet}`;
    }
    case 'quantum_target_selected': {
      const phase = event.phase === 'fuel_requested' ? 'fuel calc' : 'selected';
      return `Quantum target ${phase}: ${prettyClass(event.vehicle_class, lookup.vehicles)} → ${prettyClass(event.destination, lookup.locations)}`;
    }
    case 'attachment_received':
      return `Attached ${prettyClass(event.item_class, lookup.items)} to ${event.port}`;
    case 'vehicle_stowed': {
      const cleaned = event.landing_area
        .replace(/^\[PROC\]/, '')
        .replace(/^LandingArea_/, '');
      const label = prettyClass(cleaned, lookup.locations);
      return `Ship ${event.vehicle_id} stowed at ${label}`;
    }
    case 'burst_summary': {
      // Friendly per-rule labels for the four built-in BurstRules in
      // `crates/starstats-client/src/burst_rules.rs`. Falls back to a
      // generic "Burst" for any future remote-served rule we don't
      // know about, so the timeline never renders blank.
      const label =
        event.rule_id === 'loadout_restore_burst'
          ? 'Loadout restored'
          : event.rule_id === 'terrain_load_burst'
            ? 'Terrain loaded'
            : event.rule_id === 'hud_notification_burst'
              ? 'Notifications'
              : event.rule_id === 'vehicle_stowed_burst'
                ? 'Vehicles stowed'
                : 'Burst';
      return `${label} (${event.size} events)`;
    }
    case 'player_death': {
      // `body_class` is the player's own avatar body (e.g.
      // `body_01_noMagicPocket`), not a weapon — heuristic only,
      // no catalog covers body archetypes.
      const where = prettyClass(event.zone, lookup.locations);
      return where
        ? `Died at ${where}`
        : `Died (${toFriendlyName(event.body_class)})`;
    }
    case 'player_incapacitated': {
      const where = prettyClass(event.zone, lookup.locations);
      return where
        ? `Incapacitated at ${where}`
        : 'Incapacitated';
    }
    case 'game_crash': {
      const size = formatBytes(event.total_size_bytes);
      return `Game crashed (${event.channel}, ${size})`;
    }
    case 'launcher_activity': {
      // Lift category out of snake_case for display. Truncate long
      // messages so the timeline row stays single-line.
      const cat = event.category === 'info'
        ? null
        : event.category[0].toUpperCase() + event.category.slice(1);
      const msg = event.message.length > 120
        ? event.message.slice(0, 117) + '…'
        : event.message;
      return cat ? `Launcher · ${cat}: ${msg}` : `Launcher: ${msg}`;
    }
    case 'mission_start': {
      const kind =
        event.marker_kind === 'objective' ? 'Objective' : 'Mission';
      const name = event.mission_name?.trim();
      return name
        ? `${kind} started: ${name}`
        : `${kind} started (id ${event.mission_id.slice(0, 8)})`;
    }
    case 'mission_end': {
      const outcome = event.outcome?.trim();
      return outcome ? `Mission ended: ${outcome}` : 'Mission ended';
    }
    case 'shop_buy_request': {
      const item = prettyClass(event.item_class, lookup.items);
      const qty = event.quantity ?? null;
      if (item && qty) return `Buying ${item} × ${qty}`;
      if (item) return `Buying ${item}`;
      return 'Shop purchase requested';
    }
    case 'shop_flow_response': {
      if (event.success === true) return 'Shop purchase confirmed';
      if (event.success === false) return 'Shop purchase rejected';
      return 'Shop response received';
    }
    case 'commodity_buy_request': {
      const commodity = event.commodity
        ? toFriendlyName(event.commodity)
        : null;
      const qty = event.quantity ?? null;
      if (commodity && qty != null) {
        return `Buying ${formatQty(qty)} ${commodity}`;
      }
      if (commodity) return `Buying ${commodity}`;
      return 'Commodity purchase requested';
    }
    case 'commodity_sell_request': {
      const commodity = event.commodity
        ? toFriendlyName(event.commodity)
        : null;
      const qty = event.quantity ?? null;
      if (commodity && qty != null) {
        return `Selling ${formatQty(qty)} ${commodity}`;
      }
      if (commodity) return `Selling ${commodity}`;
      return 'Commodity sale requested';
    }
    case 'session_end': {
      const reason =
        event.kind === 'system_quit' ? 'clean quit' : 'fast shutdown';
      return `Session ended (${reason})`;
    }
    case 'remote_match':
      // Remote-served parser rules carry their own display name. If
      // the rule didn't supply one, fall back to the rule id.
      return event.event_name || `Remote rule: ${event.rule_id}`;
  }
}

/** Compact byte size for crash-dir totals. */
function formatBytes(bytes: number): string {
  if (!Number.isFinite(bytes) || bytes < 0) return '0 B';
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${Math.round(bytes / 1024)} KB`;
  if (bytes < 1024 * 1024 * 1024) {
    return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
  }
  return `${(bytes / (1024 * 1024 * 1024)).toFixed(1)} GB`;
}

/** Tabular-friendly quantity — integers stay int, floats round to
 *  one decimal so commodity terminals like "12.3 SCU" stay readable. */
function formatQty(qty: number): string {
  if (Number.isInteger(qty)) return qty.toLocaleString();
  return qty.toFixed(1);
}

function isGameEventPayload(p: unknown): p is GameEventPayload {
  return (
    typeof p === 'object' &&
    p !== null &&
    'type' in p &&
    typeof (p as { type: unknown }).type === 'string'
  );
}
