/**
 * "Contracts" section on a KB entity page — the contracts that
 * reference this vehicle / weapon / item / location.
 *
 * Extracted from the page so it can be rendered in a test without
 * standing up the page's seven dependencies (session, catalog, stats,
 * media, cohorts…). The page owns the fetch; this owns the rendering.
 */

import React from 'react';
import Link from 'next/link';
import type { Route } from 'next';
import type { ContractSummary } from '@/lib/contracts';

/** Fields that distinguish same-named contracts, joined for one line.
 *
 *  `display_name` is deliberately non-unique, so two rows here can share
 *  a title and still be different contracts. These are the values the
 *  catalogue promotes for exactly that purpose. */
function detailLine(c: ContractSummary): string {
  return [
    c.contract_type,
    c.first_step_location,
    c.required_item,
    c.step_count != null ? `${c.step_count} ${c.step_count === 1 ? 'step' : 'steps'}` : null,
  ]
    .filter(Boolean)
    .join(' · ');
}

export function RelatedContracts({ contracts }: { contracts: ContractSummary[] }) {
  // Render nothing at all rather than an empty shell — most KB entries
  // are referenced by no contract, and a permanent empty heading would
  // be noise on every one of them.
  if (contracts.length === 0) return null;

  return (
    <section style={{ marginTop: 20 }}>
      <h2
        style={{
          fontSize: 14,
          letterSpacing: '0.08em',
          textTransform: 'uppercase',
          color: 'var(--fg-dim)',
          margin: '0 0 8px',
        }}
      >
        Contracts
      </h2>
      <ul
        style={{
          listStyle: 'none',
          padding: 0,
          margin: 0,
          display: 'flex',
          flexDirection: 'column',
          gap: 6,
        }}
      >
        {contracts.map((c) => {
          const line = detailLine(c);
          return (
            <li key={c.canonical_id}>
              {/* prefetch off: KB pages are prefetch-heavy and once burst
                  the per-IP reference governor. */}
              <Link
                href={`/contracts/${c.canonical_id}` as Route}
                prefetch={false}
                className="hud-tile"
                style={{
                  display: 'flex',
                  flexDirection: 'column',
                  gap: 2,
                  padding: '8px 10px',
                  textDecoration: 'none',
                  color: 'inherit',
                }}
              >
                <span style={{ fontSize: 13, fontWeight: 600 }}>
                  {c.display_name ?? c.canonical_id}
                </span>
                {line && (
                  <span style={{ fontSize: 11, color: 'var(--fg-dim)' }}>{line}</span>
                )}
              </Link>
            </li>
          );
        })}
      </ul>
    </section>
  );
}
