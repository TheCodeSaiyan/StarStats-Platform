/**
 * Knowledge base landing. Four category tiles + a global search
 * input that links into a per-category browse with the query
 * pre-applied. Counts are derived from the same listing fetch the
 * /kb/[category] page uses, so a cold-cache landing visit is one
 * fetch per category in parallel (each hot-cached server-side).
 *
 * Server component — the listing endpoints set Next's per-fetch
 * `revalidate: 3600` so the four list fetches share the cache the
 * dashboard / journey already warm.
 */

import React from 'react';
import type { Metadata } from 'next';
import Link from 'next/link';
import type { Route } from 'next';
import { loadAllReferenceBundles } from '@/lib/reference';
import { listAllContracts } from '@/lib/contracts';
import { InstrumentStrip } from '@/components/hud/InstrumentStrip';

export const metadata: Metadata = {
  title: 'Knowledge base',
  description:
    'Browse the synced Star Citizen catalogue: ships, weapons, items, locations.',
};

const TILES: Array<{
  category: 'vehicle' | 'weapon' | 'item' | 'location';
  label: string;
  blurb: string;
}> = [
  {
    category: 'vehicle',
    label: 'Vehicles',
    blurb: 'Ships and ground vehicles from across the verse.',
  },
  {
    category: 'weapon',
    label: 'Weapons',
    blurb: 'Personal arms and ship-mounted weapons.',
  },
  {
    category: 'item',
    label: 'Items',
    blurb: 'Armor pieces, components, attachments, gear.',
  },
  {
    category: 'location',
    label: 'Locations',
    blurb: 'Systems, planets, moons, cities, outposts, stations.',
  },
];

export default async function KbLandingPage() {
  // `counts` comes pre-computed from the deduplicated entry list —
  // the `catalogs` Maps store each entry under both class_name AND
  // display_name keys to support friendly-name `<EntityLink>`
  // lookups, so `catalog.size` would render double the actual entry
  // count on the tiles.
  const { counts } = await loadAllReferenceBundles();

  // Contracts are a separate surface (not a wiki-sourced reference
  // category), so their count comes from the contracts API. Best-effort
  // — a contracts hiccup must not break the KB landing.
  let contractCount = 0;
  try {
    contractCount = (await listAllContracts()).length;
  } catch {
    contractCount = 0;
  }

  return (
    <main>
      <InstrumentStrip
        title={<h1 className="hud-tile__title" style={{ margin: 0, fontSize: 18 }}>Knowledge base</h1>}
        context="The synced Star Citizen catalogue powering names + hover stats"
        readouts={[{ k: 'entries', v: (counts.vehicle + counts.weapon + counts.item + counts.location).toLocaleString() }]}
      />

      <div
        style={{
          display: 'grid',
          gridTemplateColumns: 'repeat(auto-fit, minmax(220px, 1fr))',
          gap: 16,
          marginTop: 24,
        }}
      >
        {TILES.map((t) => (
          <Link
            key={t.category}
            href={`/kb/${t.category}` as Route}
            // No viewport prefetch: each tile's target list page fetches
            // the full category bundle (≈4 MB for vehicles), so
            // prefetching all four tiles on landing would burst ~10 MB +
            // four list renders through the rate-limited reference API.
            prefetch={false}
            className="hud-tile"
            style={{
              display: 'flex',
              flexDirection: 'column',
              gap: 6,
              padding: '14px 16px',
              textDecoration: 'none',
              color: 'var(--fg)',
            }}
          >
            <span style={{ fontSize: 18, fontWeight: 600 }}>{t.label}</span>
            <span style={{ fontSize: 13, color: 'var(--fg-muted)' }}>
              {t.blurb}
            </span>
            <span
              style={{
                fontSize: 11,
                color: 'var(--accent)',
                fontFamily: 'var(--font-mono)',
                marginTop: 'auto',
              }}
            >
              {counts[t.category].toLocaleString()} entries
            </span>
          </Link>
        ))}

        {/* Contracts — a separate surface (sp-ingest-sourced), not a
            wiki reference category, so it's its own tile linking to the
            dedicated browse. */}
        <Link
          href={'/contracts' as Route}
          prefetch={false}
          className="hud-tile"
          style={{
            display: 'flex',
            flexDirection: 'column',
            gap: 6,
            padding: '14px 16px',
            textDecoration: 'none',
            color: 'var(--fg)',
          }}
        >
          <span style={{ fontSize: 18, fontWeight: 600 }}>Contracts</span>
          <span style={{ fontSize: 13, color: 'var(--fg-muted)' }}>
            Bounties, deliveries, hauling, and mission contracts.
          </span>
          <span
            style={{
              fontSize: 11,
              color: 'var(--accent)',
              fontFamily: 'var(--font-mono)',
              marginTop: 'auto',
            }}
          >
            {contractCount.toLocaleString()} entries
          </span>
        </Link>
      </div>

      <p
        style={{
          marginTop: 28,
          fontSize: 11,
          color: 'var(--fg-muted)',
          lineHeight: 1.6,
          maxWidth: 640,
        }}
      >
        Catalogue names, specifications, and taxonomy are © Cloud Imperium
        Rights LLC / Cloud Imperium Rights Ltd. StarStats is an unofficial
        fan site, not endorsed by or affiliated with Cloud Imperium Games
        or Roberts Space Industries. Only factual data (names, specs,
        classification) is redistributed here — no third-party
        descriptive text; see{' '}
        <Link
          href={'/about#community-data-sources' as Route}
          style={{ color: 'inherit', textDecoration: 'underline' }}
        >
          /about
        </Link>{' '}
        for details.
      </p>
    </main>
  );
}
