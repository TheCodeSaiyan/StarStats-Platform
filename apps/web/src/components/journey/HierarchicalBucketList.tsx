/**
 * Two-level expandable bucket list for journey stats roll-ups.
 *
 * Server-rendered — uses native `<details>`/`<summary>` for the
 * collapse mechanic so no client JS ships. Layout:
 *
 *   ▸ Klaus & Werner                          237 ▮▮▮▮▮▮▮▮▮▮
 *       Laser Cannon  [S1 ×12] [S2 ×142]      154 ▮▮▮▮▮▮
 *       Laser Repeater                         75 ▮▮▮
 *     Behring                                   88 ▮▮▮
 *       P4-AR                                   88 ▮▮▮
 *
 * The component is generic over the row shape and renders up to three
 * levels; callers pre-roll the buckets into a `RollupNode` tree and hand
 * it in. `rollUpWeapons` (mfr → family, with size badges) is the only
 * such helper that ships today — sibling roll-ups for items and for
 * locations were deleted once the projection redesign left them without
 * callers, locations because `aggregateLocationBuckets` covers that shape
 * properly. Nothing currently produces a third level.
 */

import React from 'react';
import Link from 'next/link';
import type { Route } from 'next';
import { parseWeaponClass } from '@/lib/class-name-parts';
import { prettyClass } from '@/lib/reference';
import type { ReferenceCatalog, ReferenceMap } from '@/lib/reference';

export interface RollupNode {
  /** Display label for this node (e.g. "Klaus & Werner"). */
  label: string;
  /** Cumulative count across this node and all its children. */
  count: number;
  /** Optional badges to append to the label row, e.g. size tags. */
  badges?: Array<{ text: string; count: number }>;
  /** Optional child nodes. When present, the row becomes an
   *  expandable `<details>`. */
  children?: RollupNode[];
  /** Optional tooltip — usually the raw class identifier so power
   *  users can cross-reference the wiki. */
  title?: string;
  /** Optional inline subtitle rendered under the label. Used by the
   *  `Other / unmapped` location bucket to expose the raw class
   *  identifier on the page so we can see exactly what isn't being
   *  recognised by the parser. */
  subtitle?: string;
  /** When true, the `<details>` for this node renders open on first
   *  load. Used to surface diagnostic groupings (`Other / unmapped`)
   *  so users don't need to click to see what's inside. */
  defaultOpen?: boolean;
  /** When set, the leaf label is wrapped in a `<Link>` to the KB
   *  detail page. Populated by the rollup helpers only when the
   *  leaf represents exactly one wiki-known class AND the catalog
   *  has resolved a slug for it. Aggregate-of-multiple buckets are
   *  left as plain text — wrapping them in an entity link would
   *  imply a single entity that doesn't exist. */
  entityHref?: Route;
}

export function HierarchicalBucketList({
  nodes,
  maxNodes,
}: {
  nodes: RollupNode[];
  /** Cap the number of top-level rows shown (a widget tile never
   *  scrolls — the remainder is signalled as a "+N more" note so the
   *  full depth lives on the linked detail page). Undefined = show all
   *  (detail pages that own their own scroll pass no cap). */
  maxNodes?: number;
}) {
  const max = Math.max(...nodes.map((n) => n.count), 1);
  const shown = typeof maxNodes === 'number' ? nodes.slice(0, maxNodes) : nodes;
  const hidden = nodes.length - shown.length;
  return (
    <ol
      style={{
        listStyle: 'none',
        margin: 0,
        padding: 0,
        display: 'flex',
        flexDirection: 'column',
        gap: 6,
      }}
    >
      {shown.map((n, i) => (
        <li key={`${n.label}-${i}`}>
          <TopRow node={n} max={max} />
        </li>
      ))}
      {hidden > 0 && (
        <li>
          <span className="hud-note" style={{ margin: 0 }}>
            +{hidden.toLocaleString()} more
          </span>
        </li>
      )}
    </ol>
  );
}

function TopRow({ node, max }: { node: RollupNode; max: number }) {
  const hasChildren = !!node.children && node.children.length > 0;
  const pct = (node.count / max) * 100;
  if (!hasChildren) {
    return (
      <BarRow
        label={node.label}
        count={node.count}
        pct={pct}
        badges={node.badges}
        title={node.title}
        subtitle={node.subtitle}
        entityHref={node.entityHref}
      />
    );
  }
  const childMax = Math.max(...node.children!.map((c) => c.count), 1);
  return (
    <details open={node.defaultOpen ?? undefined}>
      <summary style={{ cursor: 'pointer', listStyle: 'none' }}>
        <BarRow
          label={node.label}
          count={node.count}
          pct={pct}
          badges={node.badges}
          title={node.title}
        subtitle={node.subtitle}
          isGroup
        />
      </summary>
      <ol
        style={{
          listStyle: 'none',
          margin: '6px 0 0 18px',
          padding: 0,
          display: 'flex',
          flexDirection: 'column',
          gap: 4,
          borderLeft: '1px solid var(--border)',
          paddingLeft: 10,
        }}
      >
        {node.children!.map((child, i) => (
          <li key={`${child.label}-${i}`}>
            <ChildRow node={child} maxChild={childMax} />
          </li>
        ))}
      </ol>
    </details>
  );
}

function ChildRow({
  node,
  maxChild,
}: {
  node: RollupNode;
  maxChild: number;
}) {
  const hasChildren = !!node.children && node.children.length > 0;
  const pct = (node.count / maxChild) * 100;
  if (!hasChildren) {
    return (
      <BarRow
        label={node.label}
        count={node.count}
        pct={pct}
        badges={node.badges}
        title={node.title}
        subtitle={node.subtitle}
        entityHref={node.entityHref}
        compact
      />
    );
  }
  const grandMax = Math.max(...node.children!.map((c) => c.count), 1);
  return (
    <details open={node.defaultOpen ?? undefined}>
      <summary style={{ cursor: 'pointer', listStyle: 'none' }}>
        <BarRow
          label={node.label}
          count={node.count}
          pct={pct}
          badges={node.badges}
          title={node.title}
        subtitle={node.subtitle}
          isGroup
          compact
        />
      </summary>
      <ol
        style={{
          listStyle: 'none',
          margin: '4px 0 0 14px',
          padding: 0,
          display: 'flex',
          flexDirection: 'column',
          gap: 3,
          borderLeft: '1px solid var(--border)',
          paddingLeft: 10,
        }}
      >
        {node.children!.map((grand, i) => {
          const gpct = (grand.count / grandMax) * 100;
          return (
            <li key={`${grand.label}-${i}`}>
              <BarRow
                label={grand.label}
                count={grand.count}
                pct={gpct}
                badges={grand.badges}
                title={grand.title}
                subtitle={grand.subtitle}
                entityHref={grand.entityHref}
                compact
              />
            </li>
          );
        })}
      </ol>
    </details>
  );
}

function BarRow({
  label,
  count,
  pct,
  badges,
  title,
  subtitle,
  entityHref,
  isGroup = false,
  compact = false,
}: {
  label: string;
  count: number;
  pct: number;
  badges?: Array<{ text: string; count: number }>;
  title?: string;
  subtitle?: string;
  /** When set, the bucket label becomes a link to the KB detail
   *  page. Populated by rollup helpers only for single-entity
   *  leaves whose class has a resolved slug. */
  entityHref?: Route;
  isGroup?: boolean;
  compact?: boolean;
}) {
  // Wrap the label in a `<Link>` when this is a single-entity leaf
  // with a resolvable KB destination. Aggregate-of-multiple buckets
  // (manufacturer / family rows) deliberately don't get the
  // treatment — clicking them shouldn't pretend to navigate to a
  // single entity.
  const labelNode = entityHref ? (
    <Link
      href={entityHref}
      className="mono"
      style={{
        color: 'var(--accent)',
        textDecoration: 'none',
        borderBottom: '1px dotted var(--accent)',
      }}
    >
      {label}
    </Link>
  ) : (
    <span className="mono">{label}</span>
  );
  return (
    <div>
      <div
        style={{
          display: 'flex',
          justifyContent: 'space-between',
          alignItems: 'baseline',
          gap: 8,
          fontSize: compact ? 11 : 12,
          marginBottom: compact ? 2 : 3,
        }}
      >
        <span
          style={{
            color: 'var(--fg)',
            overflow: 'hidden',
            display: 'flex',
            flexDirection: 'column',
            gap: 1,
          }}
          title={title}
        >
          <span
            style={{
              display: 'flex',
              gap: 6,
              alignItems: 'baseline',
              flexWrap: 'wrap',
            }}
          >
            {isGroup && (
              <span
                aria-hidden="true"
                style={{
                  color: 'var(--fg-dim)',
                  fontSize: 10,
                  lineHeight: 1,
                }}
              >
                ▸
              </span>
            )}
            {labelNode}
            {badges?.map((b) => (
              <span
                key={b.text}
                className="ss-badge"
                style={{
                  fontSize: 10,
                  padding: '1px 6px',
                  fontVariant: 'tabular-nums',
                }}
              >
                {b.text} ×{b.count}
              </span>
            ))}
          </span>
          {subtitle && (
            <span
              className="mono"
              style={{
                fontSize: 10,
                color: 'var(--fg-dim)',
                opacity: 0.75,
                overflow: 'hidden',
                textOverflow: 'ellipsis',
                whiteSpace: 'nowrap',
              }}
            >
              {subtitle}
            </span>
          )}
        </span>
        <span
          className="mono"
          style={{ color: 'var(--fg-dim)', fontVariant: 'tabular-nums' }}
        >
          {count.toLocaleString()}
        </span>
      </div>
      <div
        style={{
          height: compact ? 3 : 4,
          background: 'var(--bg-elev)',
          borderRadius: 2,
          overflow: 'hidden',
        }}
      >
        <div
          style={{
            width: `${pct}%`,
            height: '100%',
            background: 'var(--accent)',
          }}
        />
      </div>
    </div>
  );
}

// ----- Roll-up helpers --------------------------------------------

/** Helper: when a leaf bucket aggregates exactly ONE class and the
 *  rich catalog has resolved a slug for it, build the KB detail
 *  href. Returns undefined for aggregate-of-multiple buckets and
 *  for classes the catalog hasn't backfilled yet — those render
 *  as plain text. */
function leafHref(
  category: 'vehicle' | 'weapon' | 'item' | 'location',
  raws: string[],
  rich: ReferenceCatalog | undefined,
): Route | undefined {
  if (!rich || raws.length !== 1) return undefined;
  const entry = rich.get(raws[0].toLowerCase());
  if (!entry?.slug) return undefined;
  return `/kb/${category}/${entry.slug}` as Route;
}

/** Group flat `{value,count}` buckets by manufacturer → family, with
 *  per-family size badges. Used by Combat > Top weapons.
 *
 *  `catalog` is the legacy display-only map; `rich` is the optional
 *  slug-bearing catalog. When `rich` is present, single-entity leaf
 *  rows get an `entityHref` to their KB detail page. Callers that
 *  only have the display map pass `undefined` and get plain labels. */
export function rollUpWeapons(
  buckets: { value: string; count: number }[],
  catalog: ReferenceMap,
  rich?: ReferenceCatalog,
): RollupNode[] {
  const tree = new Map<
    string,
    Map<
      string,
      { count: number; sizes: Map<string, number>; raws: string[] }
    >
  >();
  for (const b of buckets) {
    const w = parseWeaponClass(b.value);
    const mfr = w.manufacturer ?? 'Unknown manufacturer';
    const family = w.family;
    let mfrMap = tree.get(mfr);
    if (!mfrMap) {
      mfrMap = new Map();
      tree.set(mfr, mfrMap);
    }
    let famEntry = mfrMap.get(family);
    if (!famEntry) {
      famEntry = { count: 0, sizes: new Map(), raws: [] };
      mfrMap.set(family, famEntry);
    }
    famEntry.count += b.count;
    famEntry.raws.push(b.value);
    if (w.size) {
      famEntry.sizes.set(w.size, (famEntry.sizes.get(w.size) ?? 0) + b.count);
    }
  }
  return [...tree.entries()]
    .map(([mfr, fams]) => {
      const children: RollupNode[] = [...fams.entries()]
        .map(([family, entry]) => {
          // If the catalog has an authoritative display for the
          // first raw class in this family, prefer it — keeps
          // wiki-aligned spellings.
          const catalogLabel = prettyClass(entry.raws[0], catalog);
          const displayFamily =
            catalogLabel && catalogLabel !== entry.raws[0]
              ? stripDuplicateMfr(catalogLabel, mfr)
              : family;
          const badges = [...entry.sizes.entries()]
            .sort((a, b) => a[0].localeCompare(b[0]))
            .map(([size, count]) => ({ text: size, count }));
          return {
            label: displayFamily,
            count: entry.count,
            badges: badges.length > 0 ? badges : undefined,
            title: entry.raws.join(' · '),
            entityHref: leafHref('weapon', entry.raws, rich),
          };
        })
        .sort((a, b) => b.count - a.count);
      const total = children.reduce((acc, c) => acc + c.count, 0);
      return {
        label: mfr,
        count: total,
        children,
      };
    })
    .sort((a, b) => b.count - a.count);
}


function stripDuplicateMfr(label: string, mfr: string): string {
  const lower = label.toLowerCase();
  const mfrLower = mfr.toLowerCase();
  if (lower.startsWith(mfrLower + ' ')) {
    return label.slice(mfr.length + 1);
  }
  return label;
}
