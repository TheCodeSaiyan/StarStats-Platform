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

/**
 * REDRAWN. Each row was a `.hud-tile` — a rounded, filled card — with four
 * inline type declarations. They are lit hairline rows now, in the same idiom
 * as the directory: the contract name is the figure, the detail line is dim.
 */
export function RelatedContracts({
  contracts,
  heading = true,
}: {
  contracts: ContractSummary[];
  /**
   * Render the component's own "Contracts" `<h2>`.
   *
   * The projection frames this in a `Pane` whose header is already an `<h2>`
   * reading "Contracts", so nesting a second one repeats it to a screen
   * reader. Defaults to true: the flat page and this component's tests keep
   * their heading. The self-gating below still applies either way — the
   * projection's section is conditioned on the same emptiness test, so a pane
   * is never rendered around nothing.
   */
  heading?: boolean;
}) {
  // Render nothing at all rather than an empty shell — most KB entries
  // are referenced by no contract, and a permanent empty heading would
  // be noise on every one of them.
  if (contracts.length === 0) return null;

  return (
    <div style={{ marginTop: 20 }}>
      {heading ? <div className="hp-fieldlabel">Contracts</div> : null}
      <ul className="hp-conlist">
        {contracts.map((c) => {
          const line = detailLine(c);
          return (
            <li key={c.canonical_id}>
              {/* prefetch off: KB pages are prefetch-heavy and once burst
                  the per-IP reference governor. */}
              <Link
                href={`/contracts/${c.canonical_id}` as Route}
                prefetch={false}
                className="hp-conlist__row"
              >
                <span className="nm">{c.display_name ?? c.canonical_id}</span>
                {line ? <span className="dt">{line}</span> : null}
              </Link>
            </li>
          );
        })}
      </ul>
    </div>
  );
}
