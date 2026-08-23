import React from 'react';
import Link from 'next/link';
import type { Route } from 'next';

/**
 * The records index, from `Records.jsx`.
 *
 * The spec puts a pilot's own records behind ONE category strip — my contracts,
 * the contract catalogue, player loadout, uploads — so they "read identically to
 * the reference surface". The product has them as four unrelated routes, and
 * before this each was a dead end: you arrived from the nav, read, and left the
 * way you came. Nothing on `/me/contracts` suggested `/me/loadout` existed.
 *
 * WHERE IT DEPARTS, and why:
 *
 *   - Entries are `Link`s, not client-state switches. These are real URLs and
 *     should be shareable and back-button correct — same reasoning as
 *     `DocsIndex`.
 *   - "Uploads" points at `/downloads`. The spec lists `/uploads`, which in the
 *     product folded into the Emitter's Uplinks group along with the whole
 *     device lifecycle; `/uploads` is a redirect now, and the system's own rule
 *     is that a redirect is never offered as a destination.
 *
 * Counts are OPTIONAL and only passed where the page already has the number.
 * A count fetched purely to decorate an index is a query per page view for a
 * figure nobody asked for.
 */
export interface RecordsIndexCounts {
  myContracts?: number;
  catalogue?: number;
  loadout?: number;
}

const ENTRIES: readonly {
  href: string;
  label: string;
  key: keyof RecordsIndexCounts | null;
}[] = [
  { href: '/me/contracts', label: 'My contracts', key: 'myContracts' },
  { href: '/contracts', label: 'Catalogue', key: 'catalogue' },
  { href: '/me/loadout', label: 'Player loadout', key: 'loadout' },
  { href: '/downloads', label: 'Uploads', key: null },
];

export function RecordsIndex({
  active,
  counts = {},
}: {
  active: string;
  counts?: RecordsIndexCounts;
}) {
  return (
    <nav className="hp-catstrip hp-recindex" aria-label="Your records">
      {ENTRIES.map((e) => {
        const n = e.key ? counts[e.key] : undefined;
        return (
          <Link
            key={e.href}
            href={e.href as Route}
            prefetch={false}
            className="hp-catchip"
            data-active={e.href === active ? 'true' : undefined}
            aria-current={e.href === active ? 'page' : undefined}
          >
            {e.label}
            {typeof n === 'number' ? (
              <b className="hp-catchip__n">{n.toLocaleString('en-GB')}</b>
            ) : null}
          </Link>
        );
      })}
    </nav>
  );
}
