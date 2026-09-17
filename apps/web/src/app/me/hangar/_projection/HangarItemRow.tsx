'use client';

import React from 'react';
import { EntityLink } from '@/components/kb/EntityLink';
import {
  resolveReferenceEntry,
  type ReferenceCategory,
  type ReferenceCatalog,
} from '@/lib/reference-types';
import { rsiStoreSearchUrl } from '@/lib/hangar-label';

export interface HangarRow {
  key: string;
  /** Display name, already prettified by `prettyHangarItem`. */
  label: string;
  /** Catalog to try, or null for something we do not classify. */
  category: ReferenceCategory | null;
  /** Bundle name for a flattened constituent, else the pledge kind. */
  note: string | null;
}

/**
 * One row of `/me/hangar`.
 *
 * Mirrors the widget's link policy deliberately, so the two surfaces
 * never disagree about where an item points: the KB link wins when the
 * reference catalog resolves the name, and anything it cannot resolve
 * — paints, flair, subscriber-store exclusives — falls back to a
 * pledge-store keyword search.
 *
 * The fallback is a SEARCH and not a product URL because store
 * availability differs per account: subscriber items are visible to
 * some readers and not others, so a URL resolved against one account
 * is not valid for another.
 */
export function HangarItemRow({
  row,
  catalogs,
}: {
  row: HangarRow;
  catalogs: {
    vehicles: ReferenceCatalog;
    weapons: ReferenceCatalog;
    items: ReferenceCatalog;
  } | null;
}) {
  const catalog =
    !catalogs || !row.category
      ? undefined
      : row.category === 'vehicle'
        ? catalogs.vehicles
        : row.category === 'weapon'
          ? catalogs.weapons
          : catalogs.items;

  const resolved = row.category
    ? resolveReferenceEntry(row.category, row.label, catalog)
    : undefined;

  const storeUrl = rsiStoreSearchUrl(row.label);

  return (
    <li className="hp-snapshots__row">
      <span>
        {row.category && resolved?.slug ? (
          <EntityLink
            category={row.category}
            classKey={row.label}
            catalog={catalog}
            label={row.label}
          />
        ) : storeUrl ? (
          <a
            href={storeUrl}
            target="_blank"
            rel="noopener noreferrer"
            title={`Find "${row.label}" in the RSI pledge store`}
          >
            {row.label}
          </a>
        ) : (
          row.label
        )}
      </span>
      {row.note ? (
        <span className="hp-snapshots__meta">{row.note}</span>
      ) : null}
    </li>
  );
}
