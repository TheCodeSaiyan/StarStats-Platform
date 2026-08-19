import 'server-only';
import React from 'react';
import { redirect } from 'next/navigation';
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
import { BodyOutline } from './_components/BodyOutline';
import { GearGroup } from './_components/GearGroup';

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
      <main className="loadout-page">
        <h1 className="loadout-page__title">Loadout</h1>
        <p className="loadout-page__empty">No loadout snapshot yet.</p>
      </main>
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

  return (
    <main className="loadout-page">
      <h1 className="loadout-page__title">Loadout</h1>

      <section className="loadout-page__armor">
        <BodyOutline slots={slots} />
      </section>

      <section className="loadout-page__gear">
        {GEAR_GROUP_ORDER.map((group) => {
          const items = gearBuckets[group] ?? [];
          return (
            <GearGroup
              key={group}
              title={GEAR_GROUP_TITLES[group]}
              items={items}
            />
          );
        })}
      </section>
    </main>
  );
}
