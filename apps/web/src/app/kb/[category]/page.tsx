/**
 * Per-category browse. Paged + searchable list of entries with a
 * per-category facet filter strip (manufacturer for vehicles +
 * weapons + items; classification + system for locations) sourced
 * from the curated `summary` projection on each entry.
 *
 * Server component. The full category catalogue is fetched once
 * (cached 1h) and search / filter is applied in-memory — fine
 * because each category tops out around ~20k entries (items) and
 * the listing is just slug + display_name + summary now.
 */

import React from 'react';
import type { Metadata } from 'next';
import Link from 'next/link';
import type { Route } from 'next';
import { notFound } from 'next/navigation';
import {
  getCategoryBundle,
  type ReferenceCategory,
  type Summary,
  placementLabel,
} from '@/lib/reference';
import { parseSortDir, sortKbEntries, type SortDir } from '@/lib/kb-sort';
import { TierChip } from '@/components/kb/TierChip';
import { InstrumentStrip } from '@/components/hud/InstrumentStrip';
import { ControlStrip } from '@/components/hud/ControlStrip';

const PAGE_SIZE = 60;

const CATEGORY_LABELS: Record<ReferenceCategory, string> = {
  vehicle: 'Vehicles',
  weapon: 'Weapons',
  item: 'Items',
  location: 'Locations',
};

const VALID: ReadonlyArray<ReferenceCategory> = [
  'vehicle',
  'weapon',
  'item',
  'location',
];

function isCategory(s: string): s is ReferenceCategory {
  return (VALID as readonly string[]).includes(s);
}

interface PageProps {
  params: Promise<{ category: string }>;
  searchParams: Promise<{
    q?: string;
    facet?: string;
    offset?: string;
    sort?: string;
    dir?: string;
  }>;
}

export async function generateMetadata(props: PageProps): Promise<Metadata> {
  const { category } = await props.params;
  if (!isCategory(category)) {
    return { title: 'Knowledge base' };
  }
  return {
    title: `${CATEGORY_LABELS[category]} — Knowledge base`,
  };
}

export default async function KbCategoryPage(props: PageProps) {
  const { category } = await props.params;
  if (!isCategory(category)) notFound();

  const {
    q = '',
    facet = '',
    offset: offsetRaw,
    sort: sortRaw,
    dir: dirRaw,
  } = await props.searchParams;
  const offset = parsePositiveInt(offsetRaw);

  // `list` holds one entry per class_name; the `catalog` map stores
  // each entry under both class_name AND display_name keys to support
  // friendly-name lookups in `<EntityLink>`, so iterating
  // `catalog.values()` would duplicate the listing.
  const { list } = await getCategoryBundle(category);
  const all = [...list];

  // In-memory filter — substring on display_name + class_name, plus
  // the chosen facet value (if any). Facets read off the typed
  // summary via `summaryFacetValue`, which type-narrows on
  // `summary.category` for each known facet key.
  const facetKey = facetKeyFor(category);
  const qLower = q.trim().toLowerCase();
  const filtered = all.filter((e) => {
    if (qLower) {
      const hay =
        e.display_name.toLowerCase() + ' ' + e.class_name.toLowerCase();
      if (!hay.includes(qLower)) return false;
    }
    if (facet && facetKey) {
      const v = summaryFacetValue(e.summary, facetKey);
      if (v !== facet) return false;
    }
    return true;
  });

  // Sort: 'name' (default) or the category's facet key (manufacturer /
  // classification), either direction. URL-param driven like q/facet so
  // the order survives navigation and pagination. Default stays
  // name-ascending, matching the prior fixed behaviour.
  const sortKey = sortRaw && facetKey && sortRaw === facetKey ? facetKey : 'name';
  const dir: SortDir = parseSortDir(dirRaw);
  const primaryValue = (e: (typeof all)[number]): string =>
    sortKey === 'name'
      ? e.display_name
      : (summaryFacetValue(e.summary, sortKey) ?? '');
  const sorted = sortKbEntries(filtered, primaryValue, dir);

  const page = sorted.slice(offset, offset + PAGE_SIZE);
  const totalPages = Math.ceil(filtered.length / PAGE_SIZE);
  const currentPage = Math.floor(offset / PAGE_SIZE) + 1;

  // Build facet chip set from the unfiltered catalogue so the user
  // can see all possible values even when they've narrowed by query.
  const facetValues = facetKey
    ? Array.from(
        new Set(
          all
            .map((e) => summaryFacetValue(e.summary, facetKey))
            .filter((v): v is string => typeof v === 'string' && v.length > 0),
        ),
      ).sort()
    : [];

  const buildHref = (newOffset: number, overrides: Record<string, string | undefined> = {}): Route => {
    const qs = new URLSearchParams();
    if (q) qs.set('q', q);
    if (facet) qs.set('facet', facet);
    // Only non-default sort state rides in the URL (keeps shareable
    // links clean). Overrides below can still flip them.
    if (sortKey !== 'name') qs.set('sort', sortKey);
    if (dir !== 'asc') qs.set('dir', dir);
    for (const [k, v] of Object.entries(overrides)) {
      if (v === undefined || v === '') qs.delete(k);
      else qs.set(k, v);
    }
    if (newOffset > 0) qs.set('offset', String(newOffset));
    else qs.delete('offset');
    const s = qs.toString();
    return (s ? `/kb/${category}?${s}` : `/kb/${category}`) as Route;
  };

  return (
    <main>
      <Link
        href={'/kb' as Route}
        style={{
          fontSize: 13,
          color: 'var(--accent)',
          textDecoration: 'none',
        }}
      >
        ← Knowledge base
      </Link>

      <InstrumentStrip
        title={<h1 className="hud-tile__title" style={{ margin: 0, fontSize: 18 }}>{CATEGORY_LABELS[category]}</h1>}
        context={`${filtered.length.toLocaleString()} entries${q ? ` matching "${q}"` : ''}${facet ? ` · ${facet}` : ''}`}
      />

      <ControlStrip>
        <form
        method="GET"
        action={`/kb/${category}`}
        style={{ display: 'flex', gap: 8, marginTop: 16, flexWrap: 'wrap' }}
      >
        <input
          type="search"
          name="q"
          defaultValue={q}
          placeholder="Search class name or display name…"
          autoComplete="off"
          style={{
            flex: '1 1 280px',
            padding: '8px 12px',
            background: 'var(--bg-elev)',
            border: '1px solid var(--border)',
            borderRadius: 'var(--r-sm)',
            color: 'var(--fg)',
            fontSize: 13,
          }}
        />
        {facet && <input type="hidden" name="facet" value={facet} />}
        {/* Preserve sort across a GET search submit (the form only
            carries its own fields, so non-default sort would reset). */}
        {sortKey !== 'name' && (
          <input type="hidden" name="sort" value={sortKey} />
        )}
        {dir !== 'asc' && <input type="hidden" name="dir" value={dir} />}
        <button type="submit" className="ss-btn ss-btn--primary">
          Search
        </button>
        {(q || facet) && (
          <Link
            href={`/kb/${category}` as Route}
            className="ss-btn ss-btn--ghost"
            // No prefetch: re-fetches the full category bundle on the
            // rate-limited reference API; click navigation is enough.
            prefetch={false}
            style={{ textDecoration: 'none' }}
          >
            Clear
          </Link>
        )}
      </form>

      {facetValues.length > 1 && facetKey && (
        <div
          style={{
            display: 'flex',
            flexWrap: 'wrap',
            gap: 6,
            marginTop: 12,
          }}
          aria-label={`Filter by ${facetKey}`}
        >
          {facetValues.map((v) => {
            const active = facet === v;
            return (
              <Link
                key={v}
                href={buildHref(0, { facet: active ? '' : v })}
                style={{
                  fontSize: 11,
                  padding: '4px 8px',
                  borderRadius: 999,
                  border: `1px solid ${active ? 'var(--accent)' : 'var(--border)'}`,
                  background: active ? 'var(--accent)' : 'transparent',
                  color: active ? 'var(--bg)' : 'var(--fg-muted)',
                  textDecoration: 'none',
                  letterSpacing: '0.02em',
                }}
              >
                {v}
              </Link>
            );
          })}
        </div>
      )}

      <div
        style={{
          display: 'flex',
          alignItems: 'center',
          flexWrap: 'wrap',
          gap: 8,
          marginTop: 12,
        }}
        aria-label="Sort entries"
      >
        <span style={{ fontSize: 11, color: 'var(--fg-dim)', letterSpacing: '0.04em' }}>
          Sort
        </span>
        {(facetKey ? ['name', facetKey] : ['name']).map((key) => {
          const active = sortKey === key;
          return (
            <Link
              key={key}
              href={buildHref(0, { sort: key === 'name' ? '' : key })}
              prefetch={false}
              data-active={active ? 'true' : undefined}
              style={{
                fontSize: 11,
                padding: '4px 10px',
                borderRadius: 999,
                border: `1px solid ${active ? 'var(--accent)' : 'var(--border)'}`,
                background: active ? 'var(--accent)' : 'transparent',
                color: active ? 'var(--bg)' : 'var(--fg-muted)',
                textDecoration: 'none',
                letterSpacing: '0.02em',
              }}
            >
              {sortKeyLabel(key)}
            </Link>
          );
        })}
        <Link
          href={buildHref(0, { dir: dir === 'asc' ? 'desc' : 'asc' })}
          prefetch={false}
          aria-label={
            dir === 'asc' ? 'Sort descending' : 'Sort ascending'
          }
          title={dir === 'asc' ? 'Ascending — click for descending' : 'Descending — click for ascending'}
          style={{
            fontSize: 11,
            padding: '4px 10px',
            borderRadius: 999,
            border: '1px solid var(--border-strong)',
            background: 'transparent',
            color: 'var(--fg-muted)',
            textDecoration: 'none',
            letterSpacing: '0.02em',
          }}
        >
          {dir === 'asc' ? '↑ A–Z' : '↓ Z–A'}
        </Link>
      </div>
      </ControlStrip>

      <section
        style={{
          display: 'grid',
          gridTemplateColumns: 'repeat(auto-fit, minmax(220px, 1fr))',
          gap: 12,
          marginTop: 16,
        }}
      >
        {page.length === 0 ? (
          <p style={{ color: 'var(--fg-dim)', fontSize: 13 }}>
            No entries match this filter.
          </p>
        ) : (
          page.map((e) => {
            // No slug → no detail page; skip the link wrapper so we
            // don't ship a dead route, but still surface the entry.
            const card = (
              <article
                className="hud-tile"
                style={{
                  padding: '10px 12px',
                  display: 'flex',
                  flexDirection: 'column',
                  gap: 3,
                  height: '100%',
                }}
              >
                <span
                  style={{
                    fontSize: 14,
                    fontWeight: 600,
                    display: 'flex',
                    alignItems: 'baseline',
                    gap: 6,
                    flexWrap: 'wrap',
                  }}
                >
                  {e.display_name}
                  {e.summary.category === 'location' && e.summary.tier && (
                    <TierChip
                      tier={e.summary.tier}
                      subtype={e.summary.subtype}
                    />
                  )}
                </span>
                <span
                  className="mono"
                  style={{
                    fontSize: 11,
                    color: 'var(--fg-dim)',
                    wordBreak: 'break-word',
                  }}
                >
                  {e.class_name}
                </span>
                {summaryPreviewFields(e.summary)
                  .slice(0, 3)
                  .map(([k, v]) => (
                    <span
                      key={k}
                      style={{ fontSize: 11, color: 'var(--fg-muted)' }}
                    >
                      <span style={{ color: 'var(--fg-dim)' }}>{k}: </span>
                      {v}
                    </span>
                  ))}
              </article>
            );
            if (!e.slug) return <div key={e.class_name}>{card}</div>;
            // `aria-label` is load-bearing: the card contents live
            // inside `<article>`, which is an ARIA landmark, and the
            // accessible-name algorithm stops at landmark boundaries
            // — without this, the link reports as nameless to screen
            // readers AND to `getByRole('link', { name: ... })`
            // assertions in e2e tests.
            return (
              <Link
                key={e.class_name}
                href={`/kb/${category}/${e.slug}` as Route}
                aria-label={e.display_name}
                // Disable viewport prefetch: a full list page has ~60
                // visible cards, and Next would prefetch every detail
                // route at once — a burst of `/slug/` SSR renders from
                // the web container's single IP that trips the API's
                // per-IP reference rate limiter (429) and crashes the
                // prefetched detail page. Navigation on click is
                // unaffected (one request).
                prefetch={false}
                style={{ textDecoration: 'none', color: 'inherit' }}
              >
                {card}
              </Link>
            );
          })
        )}
      </section>

      {totalPages > 1 && (
        <nav
          style={{
            display: 'flex',
            justifyContent: 'space-between',
            gap: 12,
            marginTop: 20,
            flexWrap: 'wrap',
          }}
        >
          <span style={{ color: 'var(--fg-muted)', fontSize: 13 }}>
            Page {currentPage} of {totalPages}
          </span>
          <div style={{ display: 'flex', gap: 8 }}>
            {offset > 0 ? (
              <Link
                href={buildHref(Math.max(0, offset - PAGE_SIZE))}
                className="ss-btn ss-btn--ghost"
              >
                ← Prev
              </Link>
            ) : (
              <span
                className="ss-btn ss-btn--ghost"
                style={{ opacity: 0.4, pointerEvents: 'none' }}
              >
                ← Prev
              </span>
            )}
            {offset + page.length < filtered.length ? (
              <Link
                href={buildHref(offset + PAGE_SIZE)}
                className="ss-btn ss-btn--ghost"
              >
                Next →
              </Link>
            ) : (
              <span
                className="ss-btn ss-btn--ghost"
                style={{ opacity: 0.4, pointerEvents: 'none' }}
              >
                Next →
              </span>
            )}
          </div>
        </nav>
      )}
    </main>
  );
}

/** Human label for a sort-key chip. 'name' is universal; the rest are
 *  the per-category facet keys reused as sort keys. */
function sortKeyLabel(key: string): string {
  if (key === 'name') return 'Name';
  return key.charAt(0).toUpperCase() + key.slice(1);
}

/** Per-category summary field most useful as a single facet chip
 *  set. Returns null for categories without an obvious facet. */
function facetKeyFor(category: ReferenceCategory): string | null {
  switch (category) {
    case 'vehicle':
      return 'manufacturer';
    case 'weapon':
      return 'manufacturer';
    case 'item':
      return 'manufacturer';
    case 'location':
      return 'classification';
  }
}

/** Type-safe facet value lookup. The known facet keys per category
 *  are (vehicle/weapon/item: manufacturer; location: classification).
 *  Returns null if the requested key isn't part of the entry's
 *  summary variant. */
function summaryFacetValue(summary: Summary, key: string): string | null {
  if (key === 'manufacturer') {
    if (
      summary.category === 'vehicle' ||
      summary.category === 'weapon' ||
      summary.category === 'item'
    ) {
      return summary.manufacturer ?? null;
    }
    return null;
  }
  if (key === 'classification' && summary.category === 'location') {
    return summary.classification ?? null;
  }
  return null;
}

/** Per-category preview pairs (label, value) for the card snippet
 *  on the listing page. Excludes the `category` discriminator (it's
 *  redundant context). Returns pairs in display order. */
function summaryPreviewFields(summary: Summary): Array<[string, string]> {
  const out: Array<[string, string]> = [];
  const push = (label: string, value: string | undefined) => {
    if (value && value.length > 0) out.push([label, value]);
  };
  switch (summary.category) {
    case 'vehicle':
      push('manufacturer', summary.manufacturer);
      push('role', summary.role);
      push('size', summary.hull_size);
      break;
    case 'weapon':
      push('manufacturer', summary.manufacturer);
      push('size', summary.size);
      push('damage', summary.damage_type);
      break;
    case 'item':
      push('manufacturer', summary.manufacturer);
      push('type', summary.item_type);
      push('grade', summary.grade);
      break;
    case 'location':
      // Surface placement first when present — it's the most
      // distinctive in-list datum (e.g. "on Daymar" / "orbits Yela")
      // and lets the card communicate context the tier chip alone
      // can't. Tier itself renders as a chip in the card header so
      // it's omitted from the preview rows.
      if (summary.placement)
        push('placement', placementLabel(summary.placement));
      push('system', summary.system);
      push('parent', summary.parent);
      // Subtype only when it adds info beyond the chip (which already
      // shows it for full-variant). The chip is `compact: false` on
      // cards, so this is purely redundant — omit.
      // Fall back to legacy classification when no Wave 2 subtype.
      if (!summary.subtype) push('type', summary.classification);
      if (summary.operator) push('operator', summary.operator);
      break;
  }
  return out;
}


function parsePositiveInt(raw: string | undefined): number {
  if (!raw) return 0;
  const n = Number.parseInt(raw, 10);
  if (!Number.isFinite(n) || n < 0) return 0;
  return n;
}
