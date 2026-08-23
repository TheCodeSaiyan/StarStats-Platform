import 'server-only';
import React from 'react';
import { redirect } from 'next/navigation';
import { RecordsIndex } from '@/components/projection/RecordsIndex';
import { getSession } from '@/lib/session';
import {
  listEvents,
  resolveReferenceItems,
  type ResolvedItem,
} from '@/lib/api';
import {
  isExcludedPort,
  slotForClassification,
  groupForItem,
  isLoadoutBurstPayload,
  pickFullestLoadoutBurst,
  type BodySlot,
  type GearGroup as GearGroupKey,
} from '@/lib/loadout';
import { type Calibration } from 'holo';
import { navSections } from '@/lib/nav';
import { getTheme } from '@/lib/theme';
import { setCalibrationAction } from '@/app/me/_projection/actions';
import {
  LoadoutProjection,
  type LoadoutSection,
} from './_projection/LoadoutProjection';
import { Paperdoll } from './_projection/Paperdoll';
import { GearPlane } from './_projection/GearPlane';

export const metadata = { title: "Loadout" };

interface SlotEntry {
  cls: string;
  port: string;
  resolved?: ResolvedItem;
}

interface GearItem {
  cls: string;
  port: string;
  resolved?: ResolvedItem;
}

const GEAR_GROUP_ORDER: GearGroupKey[] = [
  'weapons',
  'magazines',
  'attachments',
  'throwables',
  'utility',
  'consumables',
  'other',
];

const GEAR_GROUP_TITLES: Record<GearGroupKey, string> = {
  weapons: 'Weapons',
  magazines: 'Magazines',
  attachments: 'Attachments',
  throwables: 'Throwables',
  utility: 'Utility',
  consumables: 'Consumables',
  other: 'Other',
};

export default async function LoadoutPage() {
  const session = await getSession();
  if (!session) redirect('/auth/login?next=/me/loadout');

  const token = session.token;

  // The beam for this render; falls back to the system default rather than
  // failing the page.
  let calibration: Calibration = 'terra';
  try {
    calibration = (await getTheme(token)) as Calibration;
  } catch {
    // Preference read failed; the default stands.
  }

  const nav = navSections(
    { signedIn: true, staffRoles: session.staffRoles },
    'loadout',
  );
  const shell = {
    handle: session.claimedHandle,
    calibration,
    nav,
    onCalibrate: async (id: string) => {
      'use server';
      await setCalibrationAction(id);
    },
  };

  // Fetch recent burst_summary events and pick the FULLEST loadout burst
  // (a partial re-equip emits a small burst; a full spawn emits a large
  // one — the latest is often partial, so we take the one with the most
  // items as the user's complete loadout). Filtering to burst_summary
  // reaches much further back than a raw 200-event window.
  const eventsResp = await listEvents(token, {
    event_type: 'burst_summary',
    limit: 200,
  });
  const burstEvent = pickFullestLoadoutBurst(eventsResp.events);

  if (burstEvent == null || !isLoadoutBurstPayload(burstEvent.payload)) {
    return (
      <LoadoutProjection
        {...shell}
        sections={[
          {
            id: 'kit',
            title: 'Loadout',
            group: 'kit',
            // Shipped copy, verbatim — the e2e asserts on it.
            node: <p className="hp-prose">No loadout snapshot yet.</p>,
          },
        ]}
        notice={null}
      />
    );
  }

  const rawItems = burstEvent.payload.items.filter(
    (item) => !isExcludedPort(item.port),
  );

  // Collect distinct class names for bulk resolution
  const distinctClasses = [...new Set(rawItems.map((i) => i.class))];

  let resolved: Record<string, ResolvedItem> = {};
  try {
    resolved = await resolveReferenceItems(token, distinctClasses);
  } catch {
    // Fall back to prettified names — page still renders without resolution
    resolved = {};
  }

  // Partition: armor → body slots, everything else → gear groups
  const slots: Partial<Record<BodySlot, SlotEntry>> = {};
  const gearBuckets: Partial<Record<GearGroupKey, GearItem[]>> = {};

  for (const item of rawItems) {
    const resolvedItem = resolved[item.class];
    const slot = slotForClassification(resolvedItem?.classification ?? undefined);

    if (slot != null) {
      // Armor item — put in paperdoll slot (last write wins per slot)
      slots[slot] = { cls: item.class, port: item.port, resolved: resolvedItem };
    } else {
      // Carried gear — bucket by group
      const group = groupForItem(
        resolvedItem?.classification ?? undefined,
        item.port,
        item.category,
      );
      const bucket = gearBuckets[group] ?? [];
      bucket.push({ cls: item.class, port: item.port, resolved: resolvedItem });
      gearBuckets[group] = bucket;
    }
  }

  const sections: LoadoutSection[] = [
    {
      id: 'armour',
      title: 'Armour',
      ctx: 'Last restored kit',
      group: 'kit',
      node: <Paperdoll slots={slots} />,
    },
    {
      id: 'carried',
      title: 'Carried',
      group: 'kit',
      node: (
        <>
          {/* `Records.jsx` puts a pilot's own records behind one category
              strip so they read as a family; the product had four unrelated
              routes, each a dead end. */}
          <RecordsIndex active="/me/loadout" />
        <>
          {GEAR_GROUP_ORDER.map((group) => (
            <GearPlane
              key={group}
              title={GEAR_GROUP_TITLES[group]}
              items={gearBuckets[group] ?? []}
            />
          ))}
        </>
        </>
      ),
    },
  ];

  return <LoadoutProjection {...shell} sections={sections} notice={null} />;
}
