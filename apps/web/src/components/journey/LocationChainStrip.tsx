/**
 * Horizontal "where was I just now?" chain. Renders the last N distinct
 * stops as connected pills with arrows between them, most-recent on the
 * right. Designed for the top of /dashboard so a user returning after
 * a break can remember where they left off at a glance.
 *
 * Server-component shape: takes pre-fetched trace entries via props so
 * the bearer token never reaches the client. Renders nothing when there
 * are zero distinct stops, matching the `LocationPill` "no recent
 * activity" convention.
 */

import React from 'react';
import type { TraceEntry } from '@/lib/api';
import type { ReferenceCatalog } from '@/lib/reference-types';
import { EntityLink } from '@/components/kb/EntityLink';
import {
  type DistinctStop,
  toDistinctStops,
  glyphFor,
  relativeAge,
} from './trail-utils';

interface Props {
  entries: TraceEntry[];
  /** How many distinct stops to surface, oldest→newest. Default 5. */
  maxStops?: number;
  /** Heading eyebrow text. Caller can swap to fit context. */
  eyebrow?: string;
  /** Optional locations catalog. When supplied, each chain node
   *  links to `/kb/location/{slug}` with the EntityHoverCard popover
   *  on hover. Pass `catalogs.locations` from the page's
   *  `loadAllReferenceBundles()` call. */
  catalog?: ReferenceCatalog;
}

export function LocationChainStrip({
  entries,
  maxStops = 5,
  eyebrow = 'Recent stops',
  catalog,
}: Props) {
  const stops = toDistinctStops(entries).slice(-maxStops);
  if (stops.length === 0) return null;

  return (
    <section
      className="ss-card"
      style={{ padding: '14px 18px', display: 'flex', flexDirection: 'column', gap: 10 }}
      aria-label="Recent in-game stops"
    >
      <div
        style={{
          display: 'flex',
          alignItems: 'baseline',
          justifyContent: 'space-between',
          gap: 12,
        }}
      >
        <span
          style={{
            fontSize: 11,
            color: 'var(--fg-dim)',
            textTransform: 'uppercase',
            letterSpacing: '0.06em',
          }}
        >
          {eyebrow}
        </span>
        <span style={{ fontSize: 11, color: 'var(--fg-dim)' }}>
          {stops.length === 1
            ? '1 stop'
            : `${stops.length} stops · oldest → newest`}
        </span>
      </div>

      <ol
        className="ss-chain"
        style={{
          listStyle: 'none',
          margin: 0,
          padding: 0,
          display: 'flex',
          flexWrap: 'wrap',
          alignItems: 'stretch',
          gap: 0,
        }}
      >
        {stops.map((s, i) => {
          const isLast = i === stops.length - 1;
          return (
            <li
              key={s.key + s.enteredAt}
              style={{
                display: 'flex',
                alignItems: 'stretch',
                minWidth: 0,
              }}
            >
              <ChainNode stop={s} latest={isLast} catalog={catalog} />
              {!isLast && <ChainArrow />}
            </li>
          );
        })}
      </ol>
    </section>
  );
}

function ChainNode({
  stop,
  latest,
  catalog,
}: {
  stop: DistinctStop;
  latest: boolean;
  catalog?: ReferenceCatalog;
}) {
  // `primaryLabel()` already picked the most-precise field (city >
  // planet > system) and stored it in `stop.label`, so that IS the raw
  // entity identifier to look up — same as the sublabel path below uses
  // `stop.sublabel`. Re-deriving `city ?? planet ?? system` here matched
  // by luck (same precedence) but would drift if primaryLabel() ever
  // changes; use the canonical field directly (L1, finishes the
  // 2026-05-22 classKey sweep). Non-catalog labels (e.g. 'In transit')
  // fall through to plain text inside EntityLink.
  return (
    <div
      className="ss-chain-node"
      data-latest={latest ? 'true' : undefined}
      title={`${stop.resolvedLabel ?? stop.label}${stop.sublabel ? ' · ' + stop.sublabel : ''} — entered ${relativeAge(stop.enteredAt)} ago`}
      style={{
        display: 'flex',
        flexDirection: 'column',
        alignItems: 'flex-start',
        gap: 2,
        padding: '8px 12px',
        borderRadius: 0,
        border: '1px solid var(--border)',
        background: latest ? 'var(--accent-soft, var(--bg-elev))' : 'var(--bg-elev)',
        borderColor: latest ? 'var(--accent)' : 'var(--border)',
        minWidth: 100,
      }}
    >
      <div
        style={{
          display: 'flex',
          alignItems: 'baseline',
          gap: 6,
          fontSize: 13,
          fontWeight: latest ? 600 : 500,
          color: 'var(--fg)',
          whiteSpace: 'nowrap',
          overflow: 'hidden',
          textOverflow: 'ellipsis',
          maxWidth: '100%',
        }}
      >
        <span aria-hidden style={{ fontSize: 12, opacity: 0.85 }}>
          {glyphFor(stop)}
        </span>
        <span>
          <EntityLink
            category="location"
            classKey={stop.label}
            catalog={catalog}
            label={stop.label}
            resolvedLabel={stop.resolvedLabel}
            resolvedSlug={stop.resolvedSlug}
          />
        </span>
      </div>
      <div style={{ display: 'flex', gap: 8, fontSize: 11, color: 'var(--fg-dim)' }}>
        {stop.sublabel && (
          <span>
            <EntityLink
              category="location"
              // `secondaryLabel()` always returns a raw field value
              // (e.planet or e.system) or null, so `stop.sublabel`
              // is itself the right classKey — using `planet ?? system`
              // had different precedence and misaligned the lookup
              // against the displayed text in the no-city + planet ≠
              // system case.
              classKey={stop.sublabel}
              catalog={catalog}
              label={stop.sublabel}
            />
          </span>
        )}
        <span className="mono" title={stop.enteredAt}>
          {relativeAge(stop.enteredAt)} ago
        </span>
      </div>
    </div>
  );
}

function ChainArrow() {
  return (
    <span
      aria-hidden
      className="ss-chain-arrow"
      style={{
        display: 'flex',
        alignItems: 'center',
        padding: '0 6px',
        color: 'var(--fg-dim)',
        fontSize: 14,
      }}
    >
      →
    </span>
  );
}
