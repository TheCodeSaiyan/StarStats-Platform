/**
 * Where-you-are-now hero card for /journey?view=location.
 *
 * Promoted from the compact `LocationPill` (which stays in use on
 * `/dashboard` as a glanceable header chip). The hero renders:
 *   - System › Planet › City breadcrumb
 *   - Live dwell ticker (client-component island) counting up from
 *     the last distinct stop's enteredAt
 *   - "Came from" trail of the last 1–2 stops so a user can orient
 *     before scanning the timeline below.
 */

import type { ResolvedLocation } from '@/lib/api';
import type { ReferenceCatalog } from '@/lib/reference-types';
import { EntityLink } from '@/components/kb/EntityLink';
import { DwellTicker } from './DwellTicker';
import { glyphFor, relativeAge, type DistinctStop } from './trail-utils';

interface Props {
  location: ResolvedLocation | null;
  /** Distinct stops oldest→newest (server order). Used for the
   *  "came from" breadcrumb AND as the live ticker anchor. */
  stops: DistinctStop[];
  /** Optional locations catalog. When supplied, the headline,
   *  breadcrumb crumbs, and "came from" trail link to
   *  `/kb/location/{slug}` with the EntityHoverCard popover. Pass
   *  `catalogs.locations` from the page's
   *  `loadAllReferenceBundles()` call. */
  catalog?: ReferenceCatalog;
}

export function LocationHero({ location, stops, catalog }: Props) {
  if (location === null && stops.length === 0) {
    return (
      <section
        className="ss-card"
        style={{ padding: '20px 24px', borderColor: 'var(--border)' }}
      >
        <span
          style={{
            fontSize: 11,
            color: 'var(--fg-dim)',
            textTransform: 'uppercase',
            letterSpacing: '0.06em',
          }}
        >
          Current location
        </span>
        <p
          style={{
            margin: '8px 0 0',
            color: 'var(--fg-muted)',
            fontSize: 14,
          }}
        >
          No recent activity. Start a session and your last-known
          location will land here.
        </p>
      </section>
    );
  }

  const latestStop = stops[stops.length - 1];
  const headline =
    location?.city ??
    location?.planet ??
    location?.system ??
    latestStop?.label ??
    'In transit';
  const headlineKey =
    location?.city ??
    location?.planet ??
    location?.system ??
    latestStop?.city ??
    latestStop?.planet ??
    latestStop?.system;
  // Server-classified friendly name + slug for the current location.
  // Prefer the current reading's classification; fall back to the
  // latest stop's. EntityLink uses resolvedLabel over the raw `label`,
  // so this is what turns `Outpost_col_m_frm_indy_001` into a real name.
  const resolvedHeadlineLabel =
    location?.resolved_location?.display_name ??
    latestStop?.resolvedLabel ??
    null;
  const resolvedHeadlineSlug =
    location?.resolved_location?.slug ?? latestStop?.resolvedSlug ?? null;
  const breadcrumb = buildBreadcrumb(location, latestStop);
  const enteredAt = latestStop?.enteredAt ?? location?.last_seen_at;
  const eventCount = latestStop?.eventCount;
  const trail = stops.slice(-3, -1);

  return (
    <section
      className="ss-card"
      style={{
        padding: '20px 24px',
        borderColor: 'var(--accent)',
        display: 'flex',
        flexDirection: 'column',
        gap: 14,
      }}
      aria-label="Current in-game location"
    >
      <div
        style={{
          display: 'flex',
          alignItems: 'center',
          gap: 16,
          flexWrap: 'wrap',
        }}
      >
        <span aria-hidden style={{ fontSize: 32, lineHeight: 1 }}>
          {glyphFor({
            city: location?.city ?? latestStop?.city ?? null,
            planet: location?.planet ?? latestStop?.planet ?? null,
          })}
        </span>
        <div
          style={{
            display: 'flex',
            flexDirection: 'column',
            gap: 4,
            flex: 1,
            minWidth: 0,
          }}
        >
          <span
            style={{
              fontSize: 11,
              color: 'var(--fg-dim)',
              textTransform: 'uppercase',
              letterSpacing: '0.08em',
            }}
          >
            You are here
          </span>
          <span
            style={{
              fontSize: 28,
              fontWeight: 600,
              letterSpacing: '-0.02em',
              color: 'var(--fg)',
              lineHeight: 1.1,
              wordBreak: 'break-word',
            }}
          >
            <EntityLink
              category="location"
              classKey={headlineKey}
              catalog={catalog}
              label={headline}
              resolvedLabel={resolvedHeadlineLabel}
              resolvedSlug={resolvedHeadlineSlug}
              showTier
            />
          </span>
          {breadcrumb.length > 0 && (
            <span
              style={{
                fontSize: 13,
                color: 'var(--fg-muted)',
                fontFamily: 'var(--font-mono, ui-monospace, monospace)',
              }}
            >
              {breadcrumb.map((crumb, i) => (
                <span key={`${crumb}-${i}`}>
                  {i > 0 && '  ›  '}
                  <EntityLink
                    category="location"
                    classKey={crumb}
                    catalog={catalog}
                    label={crumb}
                  />
                </span>
              ))}
            </span>
          )}
        </div>
      </div>

      <dl
        style={{
          display: 'grid',
          gridTemplateColumns: 'repeat(auto-fit, minmax(140px, 1fr))',
          gap: '12px 24px',
          margin: 0,
          fontSize: 12,
        }}
      >
        {enteredAt && (
          <Cell label="Here for">
            <DwellTicker enteredAt={enteredAt} />
          </Cell>
        )}
        {enteredAt && (
          <Cell label="Arrived">
            <span
              className="mono"
              title={enteredAt}
              // relativeAge() reads Date.now(); SSR and client land
              // at different instants, so crossing a minute boundary
              // between them produces a text mismatch (React error
              // #418). Same fix as the topbar chip's time spans.
              suppressHydrationWarning
            >
              {relativeAge(enteredAt)} ago
            </span>
          </Cell>
        )}
        {typeof eventCount === 'number' && eventCount > 0 && (
          <Cell label="Events here">
            <span className="mono">{eventCount.toLocaleString()}</span>
          </Cell>
        )}
        {location?.shard && (
          <Cell label="Shard">
            <span className="mono">{location.shard}</span>
          </Cell>
        )}
      </dl>

      {trail.length > 0 && (
        <div
          style={{
            display: 'flex',
            alignItems: 'center',
            gap: 8,
            paddingTop: 6,
            borderTop: '1px dashed var(--border)',
            fontSize: 12,
            color: 'var(--fg-muted)',
            flexWrap: 'wrap',
          }}
        >
          <span style={{ color: 'var(--fg-dim)' }}>came from</span>
          {trail.map((s, i) => (
            <span
              key={s.key + s.enteredAt}
              style={{
                display: 'inline-flex',
                alignItems: 'baseline',
                gap: 6,
              }}
            >
              <span style={{ color: 'var(--fg)' }}>
                <EntityLink
                  category="location"
                  // `s.label` is the canonical field primaryLabel() chose
                  // (city > planet > system) — look that up, don't
                  // re-derive the precedence (L1, finishes 2026-05-22 sweep).
                  classKey={s.label}
                  catalog={catalog}
                  label={s.label}
                  resolvedLabel={s.resolvedLabel}
                  resolvedSlug={s.resolvedSlug}
                />
              </span>
              {i < trail.length - 1 && (
                <span aria-hidden style={{ color: 'var(--fg-dim)' }}>
                  →
                </span>
              )}
            </span>
          ))}
          <span aria-hidden style={{ color: 'var(--accent)' }}>
            →
          </span>
          <span style={{ color: 'var(--accent)' }}>
            <EntityLink
              category="location"
              classKey={headlineKey}
              catalog={catalog}
              label={headline}
              resolvedLabel={resolvedHeadlineLabel}
              resolvedSlug={resolvedHeadlineSlug}
            />
          </span>
        </div>
      )}
    </section>
  );
}

function Cell({
  label,
  children,
}: {
  label: string;
  children: React.ReactNode;
}) {
  return (
    <div style={{ display: 'flex', flexDirection: 'column', gap: 2 }}>
      <dt
        style={{
          fontSize: 10,
          color: 'var(--fg-dim)',
          textTransform: 'uppercase',
          letterSpacing: '0.08em',
        }}
      >
        {label}
      </dt>
      <dd style={{ margin: 0, fontSize: 14, color: 'var(--fg)' }}>
        {children}
      </dd>
    </div>
  );
}

function buildBreadcrumb(
  loc: ResolvedLocation | null,
  stop: DistinctStop | undefined,
): string[] {
  const system = loc?.system ?? stop?.system ?? null;
  const planet = loc?.planet ?? stop?.planet ?? null;
  const city = loc?.city ?? stop?.city ?? null;
  const headline = city ?? planet ?? system;
  const crumbs: string[] = [];
  if (system && system !== headline) crumbs.push(system);
  if (planet && planet !== headline && planet !== system) crumbs.push(planet);
  if (city && city !== headline) crumbs.push(city);
  return crumbs;
}
