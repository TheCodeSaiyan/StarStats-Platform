import React from 'react';
import { SubStats } from 'holo';
import type { LeaderCard } from '@/lib/kb-compare-types';

/**
 * Superlatives across the compared set — fastest, toughest, and so on.
 *
 * REDRAWN. These were rounded filled cards on a hardcoded `#16131d` with the
 * winner's name in a literal amber. `SubStats` is the system's row of figures:
 * the value glows, the label is dim and tracked, and the winner rides along as
 * the derivation line — which is what it is. Who holds a superlative is the
 * fact; the number alone does not say it.
 */
export function ComparisonLeaderboard({ cards }: { cards: LeaderCard[] }) {
  if (cards.length === 0) return null;
  return (
    <SubStats
      items={cards.slice(0, 4).map((c) => ({
        k: c.label,
        v: c.valueText,
        sub: c.winnerName,
      }))}
    />
  );
}
