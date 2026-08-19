import React from 'react';
import type { LeaderCard } from '@/lib/kb-compare-types';

/** Superlative cards (fastest / toughest / …) for the compared set. */
export function ComparisonLeaderboard({ cards }: { cards: LeaderCard[] }) {
  if (cards.length === 0) return null;
  return (
    <div style={{ display: 'grid', gridTemplateColumns: `repeat(${Math.min(cards.length, 4)}, 1fr)`, gap: 10 }}>
      {cards.map((c) => (
        <div key={c.key} style={{ background: 'var(--surface, #16131d)', border: '1px solid var(--border, rgba(255,255,255,.07))', borderRadius: 10, padding: '11px 13px' }}>
          <div style={{ fontSize: 10, textTransform: 'uppercase', letterSpacing: '.07em', color: 'var(--fg-muted)' }}>{c.label}</div>
          <div style={{ fontSize: 16, fontWeight: 600, marginTop: 3 }}>{c.valueText}</div>
          <div style={{ fontSize: 11, color: 'var(--accent, #E8A23C)', marginTop: 2 }}>{c.winnerName}</div>
        </div>
      ))}
    </div>
  );
}
