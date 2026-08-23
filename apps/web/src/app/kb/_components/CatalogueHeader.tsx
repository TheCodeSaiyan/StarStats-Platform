import React from 'react';
import Link from 'next/link';
import type { Route } from 'next';
import { SubStats } from 'holo';

/**
 * The catalogue's fixed shell — counts across the top, categories as tabs.
 *
 * `CatalogueLayout.jsx` opens by saying the shell "is fixed and not a screen's
 * to vary", and names its first element: a header pane carrying the title, a
 * `SubStats` of category counts, and a tab row. `Catalogue.jsx` renders exactly
 * that above every state it has — browse, search, entity detail.
 *
 * THE PORT SHIPPED WITHOUT IT, and that is what made the catalogue read as
 * unported however much the browse grid changed. Two consequences, both on the
 * screens a reader actually meets first:
 *
 *   - `/kb` was a lone Plane of five list rows. It is the page the nav points
 *     at, so "Catalogue" opened onto a link list.
 *   - `/kb/[category]` had the facet chips and the sort row but no CATEGORY
 *     row, so moving from Vehicles to Weapons meant going back a level. In the
 *     kit they are tabs, side by side, always visible.
 *
 * TABS ARE LINKS, not client state. These are real routes with real URLs — the
 * same reasoning as `DocsIndex` and `RecordsIndex`. The kit switches a `cat`
 * id because it is one mock screen.
 *
 * NOT the facet chips' treatment. Facets are `.hp-catchip`, a hairline box;
 * these are the spec's underlined caps. Drawing both as chips would flatten
 * two levels of the hierarchy into one — the category you are IN versus the
 * facet you are filtering BY.
 */
export interface CatalogueCategory {
  id: string;
  label: string;
  href: string;
  count: number;
}

export function CatalogueHeader({
  categories,
  active,
}: {
  categories: CatalogueCategory[];
  /** Category id of the current view, or undefined on the landing. */
  active?: string;
}) {
  // A category with nothing behind it is not offered — the same rule the lens
  // rail follows. Contracts are the live case: they come from ingest rather
  // than the wiki sync, so the count is legitimately zero on a fresh instance.
  const live = categories.filter((c) => c.count > 0);
  if (live.length === 0) return null;

  return (
    <div className="hp-cathead">
      <SubStats
        items={live.map((c) => ({
          k: c.label,
          v: c.count.toLocaleString('en-GB'),
        }))}
      />
      <nav
        className="hp-cattabs"
        aria-label="Catalogue categories"
        // On the landing NO tab is current, and the spec's inactive colour is
        // `--dim` because it always has one that is lit. Four dim tabs on the
        // one page whose job is picking a category read as disabled text, so
        // with nothing current they all sit at the readable weight.
        data-nocurrent={active ? undefined : 'true'}
      >
        {live.map((c) => (
          <Link
            key={c.id}
            href={c.href as Route}
            prefetch={false}
            className="hp-cattab"
            data-active={c.id === active ? 'true' : undefined}
            aria-current={c.id === active ? 'page' : undefined}
          >
            {c.label}
          </Link>
        ))}
      </nav>
    </div>
  );
}
