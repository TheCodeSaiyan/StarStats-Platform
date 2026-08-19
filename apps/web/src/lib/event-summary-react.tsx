/**
 * ReactNode-returning mirror of `formatEventSummary` in
 * `event-summary.ts`.
 *
 * Universal (no `'use client'`): the function is a pure switch
 * returning JSX with no React state, so it can be invoked directly
 * from server components (dashboard / journey TypesTab). It DOES
 * return `<EntityLink>` instances — those are client components,
 * and server components composing client components is the
 * standard pattern (the boundary is handled by Next's bundler). The two functions must produce the same
 * visible text for every variant — but this one inserts
 * `<EntityLink>` wrappers around resolved entity identifiers so
 * the dashboard / journey UI can navigate into `/kb/{category}/{slug}`
 * (and show the hover-card on hover) without a separate post-render
 * pass.
 *
 * String callers (clipboard, share preview, OG card) continue to use
 * `formatEventSummary`. UI callers should swap to this function;
 * the parity is verified by a vitest test (`text equality between
 * the two outputs for every payload variant`).
 *
 * Field-by-field mapping: every place `formatEventSummary` calls
 * `prettyClass(raw, lookup.X)` becomes
 * `<EntityLink category="X" classKey={raw} catalog={catalogs.X} />`.
 * Catalog-miss paths fall through to the heuristic
 * (`toFriendlyName`) automatically inside `<EntityLink>`.
 */

import React from 'react';
import type { ReactNode } from 'react';
import type { ReferenceCatalogs, ReferenceLookup } from './reference-types';
import {
  EMPTY_REFERENCE_CATALOGS,
  EMPTY_REFERENCE_LOOKUP,
  prettyClass,
} from './reference-types';
import { toFriendlyName } from './heuristic-name';
import { EntityLink } from '@/components/kb/EntityLink';

// Re-export the payload union from event-summary so consumers don't
// need to import from two places. Importing the type only would
// require event-summary to export it; for now we inline the same
// discriminated-union shape — kept structurally identical and
// asserted-equal by the parity test.
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

/**
 * Minimal shape of an event's `resolved_location` (the tray's
 * fuzzy-matched location, shipped on `EventDto.resolved_location`).
 * Only the fields the render path consumes — the full wire type also
 * carries `tier`/`source`.
 */
export interface ResolvedLocationLike {
  display_name: string;
  slug?: string | null;
  system?: string | null;
}

/**
 * Props that steer a location `<EntityLink>` to prefer the tray's
 * resolution over the exact catalog lookup. Spread into every location
 * link whose field corresponds to the event's `location_raw()` — the
 * fuzzy slug links locations the catalog has no exact key for.
 */
function resolvedLocationProps(resolved?: ResolvedLocationLike | null) {
  return {
    resolvedSlug: resolved?.slug ?? undefined,
    resolvedLabel: resolved?.display_name ?? undefined,
  };
}

/**
 * Format a payload into a one-liner summary node. Falls back to a
 * generic node for unknown variants.
 *
 * `resolvedLocation` is the event's tray-resolved location (when
 * present). It's applied to the single location link that corresponds
 * to the event's `location_raw()` field so a fuzzy-matched place still
 * links into `/kb/location/{slug}` even when the exact catalog lookup
 * misses.
 */
export function renderEventSummary(
  payload: unknown,
  lookup: ReferenceLookup = EMPTY_REFERENCE_LOOKUP,
  catalogs: ReferenceCatalogs = EMPTY_REFERENCE_CATALOGS,
  resolvedLocation?: ResolvedLocationLike | null,
): ReactNode {
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
  return renderKnown(payload, lookup, catalogs, resolvedLocation);
}

function renderKnown(
  event: GameEventPayload,
  lookup: ReferenceLookup,
  catalogs: ReferenceCatalogs,
  resolvedLocation?: ResolvedLocationLike | null,
): ReactNode {
  switch (event.type) {
    case 'process_init':
      return 'Game process started';
    case 'legacy_login':
      return `Logged in as ${event.handle}`;
    case 'join_pu': {
      // String formatter conditionally drops the ` · {where}` when
      // the location doesn't resolve. Mirror that here so the React
      // version doesn't render a dangling separator with empty
      // text after it. The resolved-text check uses `prettyClass`
      // (which falls through to heuristic) — same predicate as
      // formatEventSummary.
      const whereText = prettyClass(event.location_id, lookup.locations);
      return (
        <>
          Joined PU shard {event.shard}
          {whereText && (
            <>
              {' · '}
              <EntityLink
                category="location"
                classKey={event.location_id}
                catalog={catalogs.locations}
                label={whereText}
              />
            </>
          )}
          {' '}({event.address}:{event.port})
        </>
      );
    }
    case 'change_server':
      return `Server transition: ${event.phase === 'start' ? 'starting' : 'complete'}`;
    case 'seed_solar_system':
      return `Seeded ${event.solar_system} on shard ${event.shard}`;
    case 'resolve_spawn':
      return `Spawn resolved (player ${event.player_geid}, fallback=${event.fallback})`;
    case 'actor_death': {
      const killer = toFriendlyName(event.killer);
      return (
        <>
          {event.victim} killed by {killer} (
          <EntityLink
            category="weapon"
            classKey={event.weapon}
            catalog={catalogs.weapons}
          />
          , {event.damage_type})
        </>
      );
    }
    case 'vehicle_destruction':
      return (
        <>
          Vehicle destroyed:{' '}
          <EntityLink
            category="vehicle"
            classKey={event.vehicle_class}
            catalog={catalogs.vehicles}
          />{' '}
          (level {event.destroy_level}, by {event.caused_by})
        </>
      );
    case 'hud_notification':
      return `HUD: ${event.text.replace(/:\s*$/, '').replace(/:$/, '')}`;
    case 'location_inventory_requested':
      if (event.location === 'INVALID_LOCATION_ID') {
        return `${event.player} opened inventory (no location bound yet)`;
      }
      return (
        <>
          {event.player} opened inventory at{' '}
          <EntityLink
            category="location"
            classKey={event.location}
            catalog={catalogs.locations}
            {...resolvedLocationProps(resolvedLocation)}
          />
        </>
      );
    case 'planet_terrain_load': {
      const label = prettyClass(event.planet, lookup.locations) || event.planet;
      return (
        <>
          Near planet/moon:{' '}
          <EntityLink
            category="location"
            classKey={event.planet}
            catalog={catalogs.locations}
            label={label}
            {...resolvedLocationProps(resolvedLocation)}
          />
        </>
      );
    }
    case 'quantum_target_selected': {
      const phase = event.phase === 'fuel_requested' ? 'fuel calc' : 'selected';
      return (
        <>
          Quantum target {phase}:{' '}
          <EntityLink
            category="vehicle"
            classKey={event.vehicle_class}
            catalog={catalogs.vehicles}
          />{' '}
          →{' '}
          <EntityLink
            category="location"
            classKey={event.destination}
            catalog={catalogs.locations}
            {...resolvedLocationProps(resolvedLocation)}
          />
        </>
      );
    }
    case 'attachment_received':
      return (
        <>
          Attached{' '}
          <EntityLink
            category="item"
            classKey={event.item_class}
            catalog={catalogs.items}
          />{' '}
          to {event.port}
        </>
      );
    case 'vehicle_stowed': {
      const cleaned = event.landing_area
        .replace(/^\[PROC\]/, '')
        .replace(/^LandingArea_/, '');
      return (
        <>
          Ship {event.vehicle_id} stowed at{' '}
          <EntityLink
            category="location"
            classKey={cleaned}
            catalog={catalogs.locations}
            {...resolvedLocationProps(resolvedLocation)}
          />
        </>
      );
    }
    case 'burst_summary': {
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
      const zoneText = prettyClass(event.zone, lookup.locations);
      if (zoneText) {
        return (
          <>
            Died at{' '}
            <EntityLink
              category="location"
              classKey={event.zone ?? null}
              catalog={catalogs.locations}
              {...resolvedLocationProps(resolvedLocation)}
            />
          </>
        );
      }
      return `Died (${toFriendlyName(event.body_class)})`;
    }
    case 'player_incapacitated': {
      const zoneText = prettyClass(event.zone, lookup.locations);
      if (zoneText) {
        return (
          <>
            Incapacitated at{' '}
            <EntityLink
              category="location"
              classKey={event.zone ?? null}
              catalog={catalogs.locations}
              {...resolvedLocationProps(resolvedLocation)}
            />
          </>
        );
      }
      return 'Incapacitated';
    }
    case 'game_crash':
      return `Game crashed (${event.channel}, ${formatBytes(event.total_size_bytes)})`;
    case 'launcher_activity': {
      const cat =
        event.category === 'info'
          ? null
          : event.category[0].toUpperCase() + event.category.slice(1);
      const msg =
        event.message.length > 120
          ? event.message.slice(0, 117) + '…'
          : event.message;
      return cat ? `Launcher · ${cat}: ${msg}` : `Launcher: ${msg}`;
    }
    case 'mission_start': {
      const kind = event.marker_kind === 'objective' ? 'Objective' : 'Mission';
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
      const itemLabel = prettyClass(event.item_class, lookup.items);
      const qty = event.quantity ?? null;
      if (itemLabel && qty) {
        return (
          <>
            Buying{' '}
            <EntityLink
              category="item"
              classKey={event.item_class ?? null}
              catalog={catalogs.items}
              label={itemLabel}
            />{' '}
            × {qty}
          </>
        );
      }
      if (itemLabel) {
        return (
          <>
            Buying{' '}
            <EntityLink
              category="item"
              classKey={event.item_class ?? null}
              catalog={catalogs.items}
              label={itemLabel}
            />
          </>
        );
      }
      return 'Shop purchase requested';
    }
    case 'shop_flow_response':
      if (event.success === true) return 'Shop purchase confirmed';
      if (event.success === false) return 'Shop purchase rejected';
      return 'Shop response received';
    case 'commodity_buy_request': {
      const commodity = event.commodity ? toFriendlyName(event.commodity) : null;
      const qty = event.quantity ?? null;
      if (commodity && qty != null) {
        return `Buying ${formatQty(qty)} ${commodity}`;
      }
      if (commodity) return `Buying ${commodity}`;
      return 'Commodity purchase requested';
    }
    case 'commodity_sell_request': {
      const commodity = event.commodity ? toFriendlyName(event.commodity) : null;
      const qty = event.quantity ?? null;
      if (commodity && qty != null) {
        return `Selling ${formatQty(qty)} ${commodity}`;
      }
      if (commodity) return `Selling ${commodity}`;
      return 'Commodity sale requested';
    }
    case 'session_end':
      return `Session ended (${
        event.kind === 'system_quit' ? 'clean quit' : 'fast shutdown'
      })`;
    case 'remote_match':
      return event.event_name || `Remote rule: ${event.rule_id}`;
  }
}

function formatBytes(bytes: number): string {
  if (!Number.isFinite(bytes) || bytes < 0) return '0 B';
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${Math.round(bytes / 1024)} KB`;
  if (bytes < 1024 * 1024 * 1024) {
    return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
  }
  return `${(bytes / (1024 * 1024 * 1024)).toFixed(1)} GB`;
}

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
