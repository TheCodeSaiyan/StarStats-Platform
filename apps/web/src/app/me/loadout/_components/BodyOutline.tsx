import React from 'react';
import { ItemTile } from './ItemTile';
import { ItemImage } from './ItemImage';
import type { ResolvedItem } from '@/lib/api';
import type { BodySlot } from '@/lib/loadout';

interface SlotEntry {
  cls: string;
  port: string;
  resolved?: ResolvedItem;
}

interface BodyOutlineProps {
  slots: Partial<Record<BodySlot, SlotEntry>>;
}

/** Human-readable label for each body slot, shown in empty placeholders. */
const SLOT_LABELS: Record<BodySlot, string> = {
  head: 'Head',
  torso: 'Torso',
  arms: 'Arms',
  legs: 'Legs',
  undersuit: 'Undersuit',
  back: 'Back',
};

function SlotCell({ slot, entry }: { slot: BodySlot; entry: SlotEntry | undefined }) {
  if (entry != null) {
    return (
      <div className={`body-outline__slot body-outline__slot--${slot}`}>
        <ItemTile cls={entry.cls} port={entry.port} resolved={entry.resolved} />
      </div>
    );
  }
  return (
    <div className={`body-outline__slot body-outline__slot--${slot} body-outline__slot--empty`}>
      <span className="body-outline__slot-label">{SLOT_LABELS[slot]}</span>
    </div>
  );
}

/**
 * Presentational-only mirror of a SlotCell — used for the right-hand arms column
 * in the paperdoll grid. Renders the same visual as SlotCell but omits the
 * interactive KB link so there is exactly one focusable element per equipped item.
 * The whole cell is aria-hidden to screen readers.
 */
function SlotCellMirror({ slot, entry }: { slot: BodySlot; entry: SlotEntry | undefined }) {
  if (entry != null) {
    const name = entry.resolved?.display_name ?? entry.cls;
    return (
      <div
        className={`body-outline__slot body-outline__slot--${slot}`}
        aria-hidden="true"
      >
        <div className="loadout-item-tile">
          {entry.resolved?.has_image === true && (
            <ItemImage
              src={`/kb/media/${entry.resolved.category}/${entry.cls}/0`}
              alt=""
            />
          )}
          <div className="loadout-item-tile__name">
            <span>{name}</span>
          </div>
        </div>
      </div>
    );
  }
  return (
    <div
      className={`body-outline__slot body-outline__slot--${slot} body-outline__slot--empty`}
      aria-hidden="true"
    >
      <span className="body-outline__slot-label">{SLOT_LABELS[slot]}</span>
    </div>
  );
}

/**
 * CSS-grid paperdoll body layout.
 *
 * Grid rows:
 *   1. Head (centred)
 *   2. Arms | Torso | Arms
 *   3. Legs (centred)
 *   4. Undersuit | Back (footer row)
 *
 * The right-hand Arms column uses SlotCellMirror (aria-hidden, no KB link)
 * to keep the visual symmetry without duplicating the interactive tile.
 */
export function BodyOutline({ slots }: BodyOutlineProps) {
  return (
    <div className="body-outline">
      {/* Row 1 — head */}
      <div className="body-outline__row body-outline__row--head">
        <SlotCell slot="head" entry={slots.head} />
      </div>

      {/* Row 2 — arms + torso + arms (right arms is presentational mirror) */}
      <div className="body-outline__row body-outline__row--mid">
        <SlotCell slot="arms" entry={slots.arms} />
        <SlotCell slot="torso" entry={slots.torso} />
        <SlotCellMirror slot="arms" entry={slots.arms} />
      </div>

      {/* Row 3 — legs */}
      <div className="body-outline__row body-outline__row--legs">
        <SlotCell slot="legs" entry={slots.legs} />
      </div>

      {/* Row 4 — undersuit + back */}
      <div className="body-outline__row body-outline__row--footer">
        <SlotCell slot="undersuit" entry={slots.undersuit} />
        <SlotCell slot="back" entry={slots.back} />
      </div>
    </div>
  );
}
