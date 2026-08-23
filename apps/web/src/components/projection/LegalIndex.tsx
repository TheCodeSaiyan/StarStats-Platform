import React from 'react';
import Link from 'next/link';
import type { Route } from 'next';

/**
 * The legal index, from `Legal.jsx`.
 *
 * The spec groups Terms / Privacy / Trust / Support behind one strip, so a
 * reader who lands on Privacy can see that Terms and Trust exist. Before this
 * each was reachable only from a footer link and led nowhere else.
 *
 * These four documents are the "read more" destination for the condensed
 * attribution `SiteLegalPlate` carries on every page: the plate states the
 * trademark line, and this is where the full text lives.
 *
 * NO COPY IS DEFINED HERE. The strip is navigation; every word of the legal
 * text stays in its own route, unaltered.
 */
const DOCS: readonly [string, string][] = [
  ['/terms', 'Terms'],
  ['/privacy', 'Privacy'],
  ['/trust', 'Trust'],
  ['/donate', 'Support'],
];

export function LegalIndex({ active }: { active: string }) {
  return (
    <nav className="hp-catstrip hp-legalindex" aria-label="Legal documents">
      {DOCS.map(([href, label]) => (
        <Link
          key={href}
          href={href as Route}
          className="hp-catchip"
          data-active={href === active ? 'true' : undefined}
          aria-current={href === active ? 'page' : undefined}
        >
          {label}
        </Link>
      ))}
    </nav>
  );
}
