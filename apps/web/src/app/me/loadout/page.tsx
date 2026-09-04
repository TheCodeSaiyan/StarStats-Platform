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
  listLoadoutBursts,
  pickLoadoutBurst,
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

/**
 * Renders an RFC3339 capture time for display.
 *
 * Fixed en-GB rather than the reader's locale: this is server-rendered, so
 * "the reader's locale" is the CI runner's, and a date that reads
 * differently between build and browser is worse than one that reads the
 * same everywhere. Returns null for a missing or unparseable stamp so the
 * caller falls back to undated copy rather than printing "Invalid Date".
 */
function formatCapture(timestamp: string): string | null {
  if (timestamp === '') return null;
  const d = new Date(timestamp);
  if (Number.isNaN(d.getTime())) return null;
  return d.toLocaleString('en-GB', {
    day: 'numeric',
    month: 'short',
    year: 'numeric',
    hour: '2-digit',
    minute: '2-digit',
    timeZone: 'UTC',
  });
}

interface LoadoutPageProps {
  searchParams: Promise<{ snapshot?: string }>;
}

export default async function LoadoutPage(props: LoadoutPageProps) {
  const { snapshot: requestedSnapshot } = await props.searchParams;
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

  // Fetch recent burst_summary events and pick the snapshot to show: the
  // one named in `?snapshot=`, else the most recent COMPLETE restore, else
  // the fullest of any kind (see `pickLoadoutBurst`). Filtering to
  // burst_summary reaches much further back than a raw 200-event window.
  const eventsResp = await listEvents(token, {
    event_type: 'burst_summary',
    limit: 200,
  });
  const snapshots = listLoadoutBursts(eventsResp.events);
  const burstEvent = pickLoadoutBurst(eventsResp.events, requestedSnapshot);
  const chosen = snapshots.find((s) => s.event === burstEvent);

  if (burstEvent == null || !isLoadoutBurstPayload(burstEvent.payload)) {
    /**
     * SAY WHICH KIND OF NOTHING THIS IS.
     *
     * "No loadout snapshot yet" was the only message, and it conflated two
     * very different situations with different remedies:
     *
     *   - No `burst_summary` events at all. The tray has not yet seen a spawn
     *     or a re-equip. The remedy is to play; nothing is broken.
     *   - Bursts exist but none carry gear. Their payloads have no
     *     `kind: 'loadout_restore'` and no `items`, which is what a tray build
     *     from before the per-item capture produces, and what the rule_id
     *     mismatch fixed in tray-v1.8.28..31 left behind on historical rows.
     *     The remedy is a re-parse (Settings → "Re-parse local store"), or an
     *     update — and no amount of playing will fix the old rows on its own.
     *
     * A reader told only "no snapshot yet" cannot tell those apart, and will
     * wait for something that is never going to arrive.
     */
    const burstsSeen = eventsResp.events.length;
    const message =
      burstsSeen === 0
        ? 'No loadout snapshot yet.'
        : `No loadout snapshot yet — ${burstsSeen} activity burst${
            burstsSeen === 1 ? '' : 's'
          } recorded, none carrying gear.`;
    return (
      <LoadoutProjection
        {...shell}
        sections={[
          {
            id: 'kit',
            title: 'Loadout',
            group: 'kit',
            node: (
              <>
                {/* The first sentence is shipped copy, verbatim — the e2e
                    asserts on it, and it stays the lead in both cases. */}
                <p className="hp-prose">{message}</p>
                {burstsSeen > 0 ? (
                  <p className="hp-prose">
                    Gear is captured from a spawn or re-equip. Bursts recorded
                    by an older tray build carry no per-item detail — run
                    <strong> Re-parse local store</strong> in the tray&rsquo;s
                    Settings to rebuild them, then sync.
                  </p>
                ) : (
                  <p className="hp-prose">
                    The tray records your kit the next time you spawn or
                    re-equip. Nothing to do but fly.
                  </p>
                )}
              </>
            ),
          },
        ]}
        notice={null}
      />
    );
  }

  const capturedAt = formatCapture(chosen?.timestamp ?? '');

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
      // ALWAYS carry the capture time when there is one. The page can only
      // show one snapshot out of many, and an undated one reads as "now" —
      // a kit from weeks ago looked current for as long as it was the
      // biggest on record, with nothing on screen to give it away.
      ctx: capturedAt !== null ? `Restored ${capturedAt}` : 'Last restored kit',
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

  // The history. Only worth a section when there is more than one snapshot
  // to move between — a single-entry picker is furniture, not a control.
  if (snapshots.length > 1) {
    sections.push({
      id: 'snapshots',
      title: 'Snapshots',
      ctx: `${snapshots.length} recorded`,
      group: 'kit',
      node: (
        <ul className="hp-snapshots">
          {snapshots.map((s) => {
            const label = formatCapture(s.timestamp) ?? 'Undated';
            const isCurrent = s.event === burstEvent;
            return (
              <li key={s.timestamp || label} className="hp-snapshots__row">
                <a
                  href={`/me/loadout?snapshot=${encodeURIComponent(s.timestamp)}`}
                  aria-current={isCurrent ? 'true' : undefined}
                >
                  {label}
                </a>
                <span className="hp-snapshots__meta">
                  {s.itemCount} item{s.itemCount === 1 ? '' : 's'}
                  {s.complete ? ' · full restore' : ' · partial'}
                </span>
              </li>
            );
          })}
        </ul>
      ),
    });
  }

  return <LoadoutProjection {...shell} sections={sections} notice={null} />;
}
