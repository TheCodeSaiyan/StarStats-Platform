import React from 'react';
import { getMyHangar } from '@/lib/api';
import { loadAllReferenceBundles, type ReferenceCatalog } from '@/lib/reference';
import { EntityLink } from '@/components/kb/EntityLink';
import { prettyHangarItem, classifyContainedItem } from '@/lib/hangar-label';
import { logger } from '@/lib/logger';
import { defineWidget } from './kit/defineWidget';
import { RankedList, ReadoutGroup, type Row } from './kit/archetypes';
import { fmtNum } from './kit/format';

/**
 * `hangar` — owner's owned-ship snapshot pushed by the tray.
 *
 * Zero-credentials invariant (server holds NO RSI cookie): the snapshot is
 * scraped and pushed by the tray app; the widget therefore offers NO
 * "refresh from server" affordance and NO detail-page link — the full fleet
 * lives in the tray, not on a server route. The compact caveat note keeps
 * that promise explicit; expanded caps the list and surfaces the remainder
 * as a plain "+N more" note (never a scrollbar, never a fabricated link).
 *
 * Migrated to the kit: `defineWidget` owns fetch/empty/gate; `RankedList` /
 * `ReadoutGroup` own the bounded presentation.
 */
interface HangarData {
  ships: ReadonlyArray<{
    name: string;
    manufacturer?: string | null;
    kind?: string | null;
    /** Constituent items for a bundle/pack pledge (RSI "Contains:" list).
     *  Empty/absent for a plain single-item pledge. */
    contains?: string[] | null;
  }>;
  vehicles: ReferenceCatalog;
  weapons: ReferenceCatalog;
  items: ReferenceCatalog;
}

const CAVEAT = 'Sync via the tray app — server holds no RSI credentials.';
const EXPANDED_CAP = 12;

/**
 * Concise value-column text for a hangar row. For a paint/skin ("Paints -
 * <ship> - <paint>") the ship it's for is the most useful context, so
 * surface the middle segment. For everything else return `null`: the tray
 * derives `manufacturer` by splitting the pledge name on `" - "`, so it's
 * just the last name segment (e.g. "Standalone Ships - Railen" →
 * manufacturer "Railen"), NOT a real manufacturer — echoing it would
 * duplicate the label the row already shows via `<RankedList>`'s
 * `.hud-trunc`. Dropping it keeps the value column honest and the row
 * short.
 */
function rowValue(name: string): string | null {
  const segs = name.split(' - ').map((s) => s.trim()).filter(Boolean);
  if (segs.length >= 3 && /paint|skin/i.test(segs[0])) return segs[1];
  return null;
}

/**
 * A concise label for a bundle, used as the value column that ties each
 * expanded constituent back to its parent pledge. Drops a trailing
 * generic packaging word ("Bundle" / "Pack" / "Set") and re-joins the
 * remaining segments with an en-dash, so "Gear - HighSec - Bundle" reads
 * as "Gear – HighSec".
 */
function bundleShortName(name: string): string {
  const segs = name.split(' - ').map((s) => s.trim()).filter(Boolean);
  if (segs.length <= 1) return name.trim();
  const last = segs[segs.length - 1];
  const body = /^(bundle|pack|set|package|combo)$/i.test(last)
    ? segs.slice(0, -1)
    : segs;
  return body.join(' – ');
}

export const hangarWidget = defineWidget<HangarData>({
  id: 'hangar',
  eyebrow: 'Hangar',
  // Hangar is owner-only by construction (RSI cookie data, sensitive).
  visibility: 'owner',
  async load(ctx) {
    if (!ctx.token) return null;
    let hangar = null;
    try {
      hangar = await getMyHangar(ctx.token);
    } catch (err) {
      logger.warn({ err, call: 'widget.hangar' }, 'fetch failed');
      return null;
    }
    // getMyHangar returns null on 404 (no snapshot yet) — surface nothing.
    const ships = hangar?.ships ?? [];
    if (ships.length === 0) return null;
    const { catalogs } = await loadAllReferenceBundles();
    return {
      ships,
      vehicles: catalogs.vehicles,
      weapons: catalogs.weapons,
      items: catalogs.items,
    };
  },
  body(data, _ctx, size) {
    if (size === 'compact') {
      return (
        <ReadoutGroup
          readouts={[{ label: 'ships', value: fmtNum(data.ships.length) }]}
          note={CAVEAT}
        />
      );
    }
    // Expanded: ship list, capped. No see-more link — hangar has no detail
    // page (zero-credentials invariant); the remainder is a plain note.
    // Raw pledge strings ("Standalone Ships - Railen") are prettified to
    // the item's own name, and ships / weapons deep-link to the KB via
    // `<EntityLink>` (cosmetics — paints, flair — stay plain text). The
    // pretty label is also the catalog key: the reference catalog is
    // dual-keyed by display_name, so passing the stripped name resolves
    // the entry; `label` is pinned so the class-id prettifier never
    // rewrites it (docs/ENGINEERING.md free-text rule).
    //
    // A bundle/pack pledge (`contains.length > 1`) is EXPANDED inline: one
    // row per real constituent item instead of the single opaque bundle
    // row, each item deep-linked to the KB via its own heuristic category
    // (`classifyContainedItem`) with the bundle name in the value column
    // to tie the items back to their parent. Single-item pledges render
    // exactly as before. The flattened list still honours `EXPANDED_CAP`.
    const rows: Row[] = [];
    data.ships.forEach((s, i) => {
      const contains = s.contains ?? [];
      if (contains.length > 1) {
        const parent = bundleShortName(s.name);
        contains.forEach((raw, j) => {
          const item = raw.trim();
          if (!item) return;
          const category = classifyContainedItem(item);
          const catalog =
            category === 'vehicle'
              ? data.vehicles
              : category === 'weapon'
                ? data.weapons
                : data.items;
          rows.push({
            key: `${s.name}-${i}-c${j}`,
            label: (
              <EntityLink
                category={category}
                classKey={item}
                catalog={catalog}
                label={item}
              />
            ),
            value: parent,
          });
        });
        return;
      }
      const { label: pretty, category } = prettyHangarItem(s.name, s.kind);
      const catalog =
        category === 'vehicle'
          ? data.vehicles
          : category === 'weapon'
            ? data.weapons
            : undefined;
      rows.push({
        key: `${s.name}-${i}`,
        label: category ? (
          <EntityLink
            category={category}
            classKey={pretty}
            catalog={catalog}
            label={pretty}
          />
        ) : (
          pretty
        ),
        value: rowValue(s.name),
      });
    });
    const hidden = Math.max(0, rows.length - EXPANDED_CAP);
    return (
      <RankedList
        rows={rows}
        cap={EXPANDED_CAP}
        note={hidden > 0 ? `+${fmtNum(hidden)} more` : undefined}
      />
    );
  },
});
