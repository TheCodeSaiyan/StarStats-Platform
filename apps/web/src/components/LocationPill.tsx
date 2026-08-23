/**
 * "You are here" pill — surfaces the most recent location reading
 * from `GET /v1/me/location/current`. Renders nothing when the
 * server returns 204 (no recent activity).
 *
 * Server-component shape: pass the resolved location (or null) in
 * via props. The fetch happens upstream where the bearer token
 * lives. Keeping the component pure makes it composable across
 * /dashboard, /metrics, and the /journey page without each one
 * needing its own fetcher.
 */

import React from 'react';
import type { ResolvedLocation } from '@/lib/api';
import type { ReferenceCatalog } from '@/lib/reference-types';
import { EntityLink } from '@/components/kb/EntityLink';

export function LocationPill({
  location,
  catalog,
}: {
  location: ResolvedLocation | null;
  /** Optional locations catalog. When supplied, the headline links
   *  to `/kb/location/{slug}` and surfaces the EntityHoverCard
   *  popover. Pass `catalogs.locations` from the page's
   *  `loadAllReferenceBundles()` call. */
  catalog?: ReferenceCatalog;
}) {
  if (location === null) {
    return null;
  }

  // Build the display label from the most precise field available.
  // City > planet > "In transit". The shard id is context/subtext only
  // — it must never be the headline because it is a raw server routing
  // identifier, not a human-readable place name.
  const headline =
    location.city ??
    location.planet ??
    'In transit';
  // Only city/planet/system are catalog-known. Shard is a server
  // routing ID with no KB entry, so we deliberately exclude it from
  // the lookup key — `EntityLink` falls through to the plain
  // headline label in that case.
  const headlineKey = location.city ?? location.planet ?? location.system;
  const subline = buildSubline(location);
  const since = formatAge(location.last_seen_at);

  return (
    <section
      className="ss-card"
      style={{
        padding: '14px 18px',
        display: 'flex',
        alignItems: 'center',
        gap: 16,
        borderColor: 'var(--accent)',
      }}
      aria-label="Current in-game location"
    >
      <span aria-hidden style={{ fontSize: 22, lineHeight: 1 }}>
        {pickGlyph(location)}
      </span>
      <div style={{ display: 'flex', flexDirection: 'column', gap: 2, flex: 1 }}>
        <span
          style={{
            fontSize: 11,
            color: 'var(--fg-dim)',
            textTransform: 'uppercase',
            letterSpacing: '0.06em',
          }}
        >
          You are here
        </span>
        <span
          className="mono"
          style={{
            fontSize: 16,
            fontWeight: 600,
            letterSpacing: '-0.01em',
            color: 'var(--fg)',
          }}
        >
          <EntityLink
            category="location"
            classKey={headlineKey}
            catalog={catalog}
            label={headline}
            showTier
          />
        </span>
        {subline && (
          <span
            style={{
              fontSize: 12,
              color: 'var(--fg-muted)',
            }}
          >
            {subline}
          </span>
        )}
      </div>
      <span
        className="mono"
        style={{
          fontSize: 11,
          color: 'var(--fg-dim)',
        }}
        title={location.last_seen_at}
        // `since` is computed from Date.now() — server-render and
        // client-hydration land at different instants, so the text
        // can differ across a minute boundary. Suppress is the
        // canonical escape hatch for "this text is intentionally
        // time-relative"; React keeps the client value and skips
        // the mismatch warning.
        suppressHydrationWarning
      >
        {since}
      </span>
    </section>
  );
}

function buildSubline(loc: ResolvedLocation): string | null {
  // When the headline is a city we add the planet + system; when it's
  // a planet we add the system. The shard (if present) is appended as
  // context at the end — it is never the headline.
  const parts: string[] = [];
  if (loc.city && loc.planet) {
    parts.push(loc.planet);
  }
  if (loc.system && loc.system !== loc.planet) {
    parts.push(loc.system);
  }
  if (loc.shard && parts.length === 0) {
    parts.push(`Shard ${loc.shard}`);
  } else if (loc.shard) {
    parts.push(`shard ${loc.shard}`);
  }
  return parts.length > 0 ? parts.join(' · ') : null;
}

function pickGlyph(loc: ResolvedLocation): string {
  // Pure cosmetic — distinguishes the three resolution sources at a
  // glance. No icon font dependency; a Unicode glyph is enough.
  if (loc.city) return '🛰';
  if (loc.planet) return '🪐';
  return '✦';
}

function formatAge(isoTimestamp: string): string {
  const ts = new Date(isoTimestamp);
  if (Number.isNaN(ts.getTime())) return '';
  const ageMs = Date.now() - ts.getTime();
  const ageMin = Math.floor(ageMs / 60_000);
  if (ageMin < 1) return 'just now';
  if (ageMin < 60) return `${ageMin}m ago`;
  const ageHr = Math.floor(ageMin / 60);
  if (ageHr < 24) return `${ageHr}h ago`;
  const ageDay = Math.floor(ageHr / 24);
  return `${ageDay}d ago`;
}

/**
 * Compact header variant — single-line "you are here" chip that sits
 * in the topbar next to the brand mark. Reads as a labelled sentence
 * with TWO separate time metrics:
 *
 *   📍 YOU ARE HERE  Orison · Crusader   here 2h 14m  · sync 2m
 *
 * Two metrics, not one, because they answer different questions:
 *  - **dwell** (`here Xh Ym`) — how long you've been at this stop,
 *    sourced from the latest distinct trace stop's `enteredAt`. Hides
 *    when the layout couldn't fetch the trace.
 *  - **sync** (`sync Xm`) — how long since the tray last fed us an
 *    event at this location (`ResolvedLocation.last_seen_at`). Colour
 *    shifts amber past 30 min and red past 24h so an idle tray
 *    doesn't masquerade as "live position".
 *
 * Styling does NOT inherit `.ss-badge` — that selector forces
 * uppercase tracking-wide on the value, which mangles proper nouns
 * like "Orison". The chip uses local pill styling instead.
 */
export function LocationChip({
  location,
  dwellStart = null,
  dwellIsLowerBound = false,
  catalog,
}: {
  location: ResolvedLocation | null;
  /** ISO 8601 timestamp the user entered this location. Sourced
   *  from `ResolvedLocation.entered_at` (computed server-side over
   *  an unbounded event run). Optional — hides the dwell tail when
   *  missing (e.g. a fresh session with only one location event). */
  dwellStart?: string | null;
  /** When true, the server marked `entered_at` as a lower bound:
   *  the walk-back exhausted its batch without finding a key change,
   *  so the real dwell may be longer. The chip appends a trailing
   *  `+` ("here 23h 57m+") so the user knows the value is "at least
   *  this long" rather than precise. */
  dwellIsLowerBound?: boolean;
  /** Optional locations catalog. When supplied, the chip's headline
   *  and the optional sub-label link to `/kb/location/{slug}` with
   *  the EntityHoverCard popover. Pass from the layout's
   *  `getCategoryBundle('location').catalog`. */
  catalog?: ReferenceCatalog;
}) {
  if (location === null) return null;
  // Shard is context/subtext only — never the headline.
  const headline =
    location.city ?? location.planet ?? 'In transit';
  const headlineKey = location.city ?? location.planet ?? location.system;
  const sub =
    location.city && location.planet
      ? location.planet
      : location.planet &&
          location.system &&
          location.system !== location.planet
        ? location.system
        : null;
  const subKey =
    location.city && location.planet
      ? location.planet
      : location.planet && location.system
        ? location.system
        : null;

  const syncMin = ageMinutes(location.last_seen_at);
  const syncColor =
    syncMin === null
      ? 'var(--fg-dim)'
      : syncMin >= 24 * 60
        ? 'var(--danger)'
        : syncMin >= 30
          ? 'var(--warn)'
          : 'var(--fg-dim)';
  const syncLabel = formatAge(location.last_seen_at);
  const dwellLabel =
    dwellStart != null
      ? `${formatDwellSince(dwellStart)}${dwellIsLowerBound ? '+' : ''}`
      : null;

  return (
    <span
      style={{
        display: 'inline-flex',
        alignItems: 'baseline',
        gap: 8,
        maxWidth: 460,
        padding: '4px 10px',
        borderRadius: 0,
        border: '1px solid color-mix(in oklab, var(--accent) 40%, transparent)',
        background:
          'var(--accent-soft, color-mix(in oklab, var(--accent) 8%, transparent))',
        fontSize: 12,
        lineHeight: 1.4,
        whiteSpace: 'nowrap',
        overflow: 'hidden',
      }}
      title={[
        `Currently at ${headline}${sub ? ` · ${sub}` : ''}`,
        dwellLabel ? `Here for ${dwellLabel}` : null,
        `Last sync ${syncLabel}`,
      ]
        .filter(Boolean)
        .join(' — ')}
      aria-label={`You are here: ${headline}${sub ? `, ${sub}` : ''}${dwellLabel ? `, here ${dwellLabel}` : ''}, last sync ${syncLabel}`}
    >
      <span aria-hidden style={{ fontSize: 12, lineHeight: 1.4 }}>
        {pickGlyph(location)}
      </span>
      <span
        style={{
          color: 'var(--fg-muted)',
          textTransform: 'uppercase',
          letterSpacing: '0.06em',
          fontSize: 10,
          fontWeight: 600,
        }}
      >
        You are here
      </span>
      <span
        style={{
          color: 'var(--fg)',
          fontWeight: 500,
          overflow: 'hidden',
          textOverflow: 'ellipsis',
          minWidth: 0,
        }}
      >
        <EntityLink
          category="location"
          classKey={headlineKey}
          catalog={catalog}
          label={headline}
          showTier
          tierChipVariant="compact"
        />
        {sub && (
          <span style={{ color: 'var(--fg-muted)', fontWeight: 400 }}>
            {' · '}
            <EntityLink
              category="location"
              classKey={subKey}
              catalog={catalog}
              label={sub}
            />
          </span>
        )}
      </span>
      {dwellLabel && (
        <span
          className="mono"
          style={{
            color: 'var(--fg-muted)',
            fontSize: 11,
            paddingLeft: 4,
            borderLeft:
              '1px solid color-mix(in oklab, var(--accent) 20%, transparent)',
          }}
          title={`Entered at ${dwellStart}`}
          // formatDwellSince() reads Date.now(); SSR and client land
          // at different instants, so the rendered text can differ.
          // Suppress is the canonical escape hatch for intentionally
          // time-relative text — React keeps the client value.
          suppressHydrationWarning
        >
          here {dwellLabel}
        </span>
      )}
      <span
        className="mono"
        style={{
          color: syncColor,
          fontSize: 11,
          paddingLeft: 4,
          borderLeft:
            '1px solid color-mix(in oklab, var(--accent) 20%, transparent)',
        }}
        title={`Last sync: ${location.last_seen_at}`}
        // Same Date.now() reason as the dwell span above. Crossing a
        // minute boundary between SSR and hydration would otherwise
        // throw React error #418.
        suppressHydrationWarning
      >
        sync {syncLabel}
      </span>
    </span>
  );
}

/**
 * Minutes since `iso`, or null when unparseable. Used to colour the
 * sync staleness tail — decoupled from the formatter so the
 * threshold checks don't re-parse the timestamp.
 */
function ageMinutes(iso: string): number | null {
  const t = new Date(iso).getTime();
  if (!Number.isFinite(t)) return null;
  return Math.max(0, Math.floor((Date.now() - t) / 60_000));
}

/**
 * Render dwell since `enteredAt` for the header chip — compact form
 * (`2h 14m`, `45m`, `12s`) without the live ticking second-by-second.
 * For continuous tick-up the journey hero uses `<DwellTicker>`; in
 * the topbar a server-render snapshot is plenty since the layout
 * re-renders on every navigation.
 */
function formatDwellSince(enteredAt: string): string {
  const t = new Date(enteredAt).getTime();
  if (!Number.isFinite(t)) return '—';
  const secs = Math.max(0, Math.floor((Date.now() - t) / 1000));
  if (secs < 60) return `${secs}s`;
  const mins = Math.floor(secs / 60);
  if (mins < 60) return `${mins}m`;
  const hours = Math.floor(mins / 60);
  const remMins = mins % 60;
  return remMins === 0 ? `${hours}h` : `${hours}h ${remMins}m`;
}
