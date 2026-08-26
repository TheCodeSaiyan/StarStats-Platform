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
  loadAllReferenceBundles,
  type ReferenceCategory,
  type Summary,
  placementLabel,
} from '@/lib/reference';
import { prettyItemType } from '@/lib/reference-types';
import { parseSortDir, sortKbEntries, type SortDir } from '@/lib/kb-sort';
import { BrowseCompare } from './_components/BrowseCompare';
import {
  CatalogueHeader,
  type CatalogueCategory,
} from '../_components/CatalogueHeader';
import { TierChip } from '@/components/kb/TierChip';
import {
  BeamInput,
  BeamButton,
  Flatline,
  type Calibration,
} from 'holo';
import { getSession } from '@/lib/session';
import { getTheme } from '@/lib/theme';
import { navSections } from '@/lib/nav';
import { setCalibrationAction } from '@/app/me/_projection/actions';
import { KbProjection, type KbSection } from '../_projection/KbProjection';

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
  // The shell's counts. `loadAllReferenceBundles` is the same build-time
  // snapshot `getCategoryBundle` reads, so this is not a second fetch — see
  // `lib/reference.ts`, which is explicit that the listing is static since M10.
  const { counts } = await loadAllReferenceBundles();
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

  // Public surface — a visitor may have no session, and the chrome renders
  // for them with the nav filtered and a Sign in action.
  const session = await getSession();
  let calibration: Calibration = 'terra';
  try {
    calibration = (await getTheme(session?.token)) as Calibration;
  } catch {
    // Preference read failed; the default stands.
  }

  const categories: CatalogueCategory[] = (
    ['vehicle', 'weapon', 'item', 'location'] as const
  ).map((c) => ({
    id: c,
    label: CATEGORY_LABELS[c],
    href: `/kb/${c}`,
    count: counts[c],
  }));

  const sections: KbSection[] = [
    {
      id: 'browse',
      title: CATEGORY_LABELS[category],
      ctx: `${filtered.length.toLocaleString()} entries${q ? ` matching “${q}”` : ''}${facet ? ` · ${facet}` : ''}`,
      group: 'kb',
      node: (
        <>
          <CatalogueHeader categories={categories} active={category} />
          {/* A plain GET form, deliberately: the browse is a URL, so a search
              is shareable, bookmarkable and back-button correct, and it works
              with JavaScript off. The hidden fields preserve facet/sort/dir
              across a submit — the form only carries its own fields, so
              without them a search would silently reset the reader's sort. */}
          <form method="GET" action={`/kb/${category}`} className="hp-formrow">
            <BeamInput
              id="kb-q"
              label="Search"
              type="search"
              name="q"
              defaultValue={q}
              placeholder="Class name or display name…"
              autoComplete="off"
            />
            {facet ? <input type="hidden" name="facet" value={facet} /> : null}
            {sortKey !== 'name' ? (
              <input type="hidden" name="sort" value={sortKey} />
            ) : null}
            {dir !== 'asc' ? <input type="hidden" name="dir" value={dir} /> : null}
            <BeamButton type="submit" variant="primary">
              Search
            </BeamButton>
            {q || facet ? (
              <Link
                href={`/kb/${category}` as Route}
                className="hp-btn hp-btn--ghost"
                // No prefetch: this re-fetches the whole category bundle from
                // the rate-limited reference API.
                prefetch={false}
              >
                Clear
              </Link>
            ) : null}
          </form>

          {/* Facets come from the UNFILTERED set, so choosing one never
              removes the others from the row — otherwise a reader who picks a
              facet can no longer see what else was available. */}
          {facetValues.length > 1 && facetKey ? (
            <nav style={{ marginTop: 16 }} aria-label={`Filter by ${facetKey}`}>
              <div className="hp-catlabel">{facetKey}</div>
              <div className="hp-catstrip">
                {facetValues.map((v) => (
                  <Link
                    key={v}
                    href={buildHref(0, { facet: facet === v ? '' : v })}
                    prefetch={false}
                    className="hp-catchip"
                    data-active={facet === v ? 'true' : undefined}
                  >
                    {v}
                  </Link>
                ))}
              </div>
            </nav>
          ) : null}

          <nav
            style={{
              display: 'flex',
              alignItems: 'center',
              gap: 8,
              marginTop: 16,
              flexWrap: 'wrap',
            }}
            aria-label="Sort entries"
          >
            <span className="hp-catlabel" style={{ margin: 0 }}>
              Sort
            </span>
            {(facetKey ? ['name', facetKey] : ['name']).map((key) => (
              <Link
                key={key}
                href={buildHref(0, { sort: key === 'name' ? '' : key })}
                prefetch={false}
                className="hp-catchip"
                aria-current={sortKey === key ? 'page' : undefined}
              >
                {sortKeyLabel(key)}
              </Link>
            ))}
            <Link
              href={buildHref(0, { dir: dir === 'asc' ? 'desc' : 'asc' })}
              prefetch={false}
              className="hp-catchip"
              aria-label={dir === 'asc' ? 'Sort descending' : 'Sort ascending'}
              title={
                dir === 'asc'
                  ? 'Ascending — click for descending'
                  : 'Descending — click for ascending'
              }
            >
              {dir === 'asc' ? '↑ A–Z' : '↓ Z–A'}
            </Link>
          </nav>

          {/* Overlay comparison — `Catalogue.jsx` puts it between the sort row
              and the grid, and the first pass at this page omitted it entirely.
              Options come from the CURRENT page, so what you can compare is
              what you are looking at. */}
          <BrowseCompare
            category={category}
            options={page
              .filter((e) => e.slug)
              .map((e) => ({ slug: e.slug as string, name: e.display_name }))}
          />

          {page.length === 0 ? (
            <Flatline
              title="No entries match this filter"
              reason="no-data"
              hint="Clear the search or pick a different facet."
            />
          ) : (
            <div className="hp-catgrid">
              {page.map((e) => {
                // The card IS a plane — `Catalogue.jsx` draws each entry as
                // `hp-plane flat`, not as a bespoke card class. The first pass
                // at this page invented `hp-kbcard` because the spec'd screen
                // was never read.
                const body = (
                  <article className="hp-plane flat hp-catcard">
                    <span className="hp-catcard__name">
                      {e.display_name}
                      {e.summary.category === 'location' && e.summary.tier ? (
                        <TierChip
                          tier={e.summary.tier}
                          subtype={e.summary.subtype}
                        />
                      ) : null}
                    </span>
                    <span className="hp-catcard__cls">{e.class_name}</span>
                    {summaryPreviewFields(e.summary)
                      .slice(0, 3)
                      .map(([k, v]) => (
                        <span className="hp-catcard__fact" key={k}>
                          <i>{k}: </i>
                          {v}
                        </span>
                      ))}
                  </article>
                );
                // No slug means no detail route — surface the entry but do not
                // ship a dead link.
                if (!e.slug) return <div key={e.class_name}>{body}</div>;
                return (
                  <Link
                    key={e.class_name}
                    href={`/kb/${category}/${e.slug}` as Route}
                    prefetch={false}
                    // LOAD-BEARING. The card's contents sit inside `<article>`,
                    // an ARIA landmark, and the accessible-name algorithm stops
                    // at landmark boundaries — without this the link reports as
                    // nameless to screen readers AND to `getByRole('link', {
                    // name })` in the e2e suite.
                    aria-label={e.display_name}
                    className="hp-catcard"
                  >
                    {body}
                  </Link>
                );
              })}
            </div>
          )}

          {totalPages > 1 ? (
            <nav className="hp-catpager" aria-label="Pagination">
              <span>
                Page {currentPage} of {totalPages}
              </span>
              <span className="hp-catpager__btns">
                {offset > 0 ? (
                  <Link
                    href={buildHref(Math.max(0, offset - PAGE_SIZE))}
                    prefetch={false}
                    className="hp-btn hp-btn--ghost"
                  >
                    ← Prev
                  </Link>
                ) : (
                  <span className="hp-btn hp-btn--ghost" aria-disabled="true">
                    ← Prev
                  </span>
                )}
                {offset + page.length < filtered.length ? (
                  <Link
                    href={buildHref(offset + PAGE_SIZE)}
                    prefetch={false}
                    className="hp-btn hp-btn--ghost"
                  >
                    Next →
                  </Link>
                ) : (
                  <span className="hp-btn hp-btn--ghost" aria-disabled="true">
                    Next →
                  </span>
                )}
              </span>
            </nav>
          ) : null}
        </>
      ),
    },
  ];

  return (
    <KbProjection
      handle={session?.claimedHandle}
      calibration={calibration}
      nav={navSections(
        { signedIn: Boolean(session), staffRoles: session?.staffRoles },
        'kb',
      )}
      crumb={[
        { label: 'Knowledge base', href: '/kb' },
        { label: CATEGORY_LABELS[category] },
      ]}
      sections={sections}
      notice={null}
      onCalibrate={async (id: string) => {
        'use server';
        await setCalibrationAction(id);
      }}
    />
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
      push('type', summary.item_type ? prettyItemType(summary.item_type) : undefined);
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
