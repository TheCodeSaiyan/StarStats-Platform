import React from 'react';
import type { ResolvedItem } from '@/lib/api';
import type { BodySlot } from '@/lib/loadout';
import { LoadoutItem } from './LoadoutItem';

/**
 * The paperdoll: armour arranged as a body, not as a list.
 *
 * The arrangement IS the information — you read "no helmet" from the gap
 * where the head should be, which a list of six labelled rows does not give
 * you. That is why this stays a bespoke layout rather than becoming a
 * `HoloKV`, and it is consistent with the system's stance that the imagery is
 * the data: no illustration, just the shape the data makes.
 *
 * The right-hand arms column is a PRESENTATIONAL MIRROR — `aria-hidden`, no
 * KB link — so the outline reads as a body without announcing the same item
 * twice to a screen reader or offering two links to one thing.
 */
export interface SlotEntry {
  cls: string;
  port: string;
  resolved?: ResolvedItem;
}

const SLOT_LABELS: Record<BodySlot, string> = {
  head: 'Head',
  torso: 'Torso',
  arms: 'Arms',
  legs: 'Legs',
  undersuit: 'Undersuit',
  back: 'Back',
};

function SlotCell({
  slot,
  entry,
}: {
  slot: BodySlot;
  entry: SlotEntry | undefined;
}) {
  if (entry == null) {
    return (
      <div className={`hp-slot hp-slot--${slot} hp-slot--empty`}>
        <span className="hp-slot__label">{SLOT_LABELS[slot]}</span>
      </div>
    );
  }
  return (
    <div className={`hp-slot hp-slot--${slot}`}>
      <span className="hp-slot__label">{SLOT_LABELS[slot]}</span>
      <LoadoutItem cls={entry.cls} port={entry.port} resolved={entry.resolved} />
    </div>
  );
}

/** Visual symmetry only — never announced, never linked. */
function SlotCellMirror({
  slot,
  entry,
}: {
  slot: BodySlot;
  entry: SlotEntry | undefined;
}) {
  return (
    <div
      className={
        entry == null
          ? `hp-slot hp-slot--${slot} hp-slot--empty`
          : `hp-slot hp-slot--${slot}`
      }
      aria-hidden="true"
    >
      <span className="hp-slot__label">{SLOT_LABELS[slot]}</span>
      {entry != null ? (
        <div className="hp-kit">
          <div className="hp-kit__name">
            <span>{entry.resolved?.display_name ?? entry.cls}</span>
          </div>
        </div>
      ) : null}
    </div>
  );
}

export function Paperdoll({
  slots,
}: {
  slots: Partial<Record<BodySlot, SlotEntry>>;
}) {
  return (
    <div className="hp-paperdoll">
      <div className="hp-paperdoll__row">
        <SlotCell slot="head" entry={slots.head} />
      </div>
      <div className="hp-paperdoll__row">
        <SlotCell slot="arms" entry={slots.arms} />
        <SlotCell slot="torso" entry={slots.torso} />
        <SlotCellMirror slot="arms" entry={slots.arms} />
      </div>
      <div className="hp-paperdoll__row">
        <SlotCell slot="legs" entry={slots.legs} />
      </div>
      <div className="hp-paperdoll__row">
        <SlotCell slot="undersuit" entry={slots.undersuit} />
        <SlotCell slot="back" entry={slots.back} />
      </div>
    </div>
  );
}
