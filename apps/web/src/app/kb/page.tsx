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
import { Plane, MeterRow, type Calibration } from 'holo';
import { getSession } from '@/lib/session';
import { getTheme } from '@/lib/theme';
import { navSections } from '@/lib/nav';
import { setCalibrationAction } from '@/app/me/_projection/actions';
import { KbProjection, type KbSection } from './_projection/KbProjection';
import { RowLink } from '@/app/me/_projection/RowLink';
import {
  CatalogueHeader,
  type CatalogueCategory,
} from './_components/CatalogueHeader';

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

  // Public surface: a visitor may have no session at all, and the chrome has
  // to render for them — nav filtered, Sign in instead of the account menu.
  const session = await getSession();
  let calibration: Calibration = 'terra';
  try {
    calibration = (await getTheme(session?.token)) as Calibration;
  } catch {
    // Preference read failed; the default stands.
  }

  const total =
    counts.vehicle + counts.weapon + counts.item + counts.location;

  // The shell's own vocabulary, shared with every category view so the tabs
  // read identically wherever you are standing.
  const categories: CatalogueCategory[] = [
    ...TILES.map((t) => ({
      id: t.category,
      label: t.label,
      href: `/kb/${t.category}`,
      count: counts[t.category],
    })),
    // Contracts are ingest-sourced rather than wiki-synced, so they browse
    // from their own route — but they are a catalogue category to a reader,
    // and leaving them out of the tab row is why `/contracts` was unreachable
    // from here.
    { id: 'contract', label: 'Contracts', href: '/contracts', count: contractCount },
  ];

  const sections: KbSection[] = [
    {
      id: 'catalogue',
      // The kit's own title and qualifier for this pane. It said "Categories",
      // which names the list rather than the thing.
      title: 'Catalogue',
      ctx: `${total.toLocaleString()} entries · wiki-synced, engine ids resolved`,
      group: 'kb',
      node: (
        <>
          <CatalogueHeader categories={categories} />
          <p className="hp-prose">
            The synced Star Citizen catalogue behind every name and hover stat
            in the projection. Pick a category above, or read what each holds.
          </p>
          <Plane tilt="flat" cap="What each category holds" style={{ marginTop: 18 }}>
            {TILES.map((t, i) => (
              <MeterRow
                key={t.category}
                rank={i + 1}
                // The ROW is the link, not the label inside it. Measured on a
                // phone these anchors were 33-58px wide and 16px tall in a
                // full-width row — the same fault reported on /me's ranked
                // planes, and the same fix: `MeterRow` renders itself as the
                // anchor when it has an href.
                name={t.label}
                href={`/kb/${t.category}`}
                linkAs={RowLink}
                // The blurb earns its place now that the counts live in the
                // header: the list answers "what is in here", not "how many".
                value={t.blurb}
                valueText
              />
            ))}
          </Plane>
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
      crumb={[{ label: 'Knowledge base' }]}
      sections={sections}
      notice={null}
      onCalibrate={async (id: string) => {
        'use server';
        await setCalibrationAction(id);
      }}
    />
  );
}
