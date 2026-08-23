import React from 'react';
import Link from 'next/link';
import type { Route } from 'next';
import { LegalPlate } from 'holo';

/**
 * The product's own attribution, in the system's `LegalPlate`.
 *
 * THIS EXISTS BECAUSE THE PORT LOST IT. The flat shell rendered `.site-footer`
 * from `layout.tsx`'s signed-out branch, so every public page carried the CIG
 * trademark line. Once each of those pages became a projection, that footer was
 * hidden by `projection-shell.css` — and only the landing page, which had been
 * given a `LegalPlate` of its own, still showed the notice. Every other public
 * surface shipped without it.
 *
 * The design system is explicit that this is not decoration: "Every static
 * surface carries the CIG trademark disclaimer verbatim." So it is a component
 * used by the shared shells rather than copy each screen retypes.
 *
 * THE SIGNED-IN FOOTER WENT THE SAME WAY. `.ss-app-footer` carried the same
 * obligation for signed-in surfaces — its own comment says "Brand book §11
 * compliance: About + Fankit + Fandom-FAQ outbound links plus the attribution
 * chip are reachable from every signed-in surface" — and it is hidden by
 * `projection-shell.css` too. So this plate is on by default in `PaneSurface`
 * rather than opt-in for public routes only, and it carries the §11 outbound
 * links as well as the trademark line.
 *
 * THE WORDS ARE THE PRODUCT'S, NOT THE KIT'S. `LegalPlate`'s built-in
 * `CIG_DISCLAIMER` is a different and shorter text; the shipped notice names
 * Squadron 42, asserts the Cloud Imperium Rights copyright over ship, vehicle,
 * weapon and item names AND specifications, and links to `/about` for the
 * data-sources statement. Legal copy is not a porter's to shorten.
 */
export function SiteLegalPlate({ version }: { version?: string }) {
  return (
    <LegalPlate
      version={version}
      licence="MPL-2.0"
      links={
        <>
          <Link href={'/about' as Route}>About</Link>
          <Link href={'/lore' as Route}>Lore</Link>
          <Link href={'/changelog' as Route}>Changelog</Link>
          <Link href={'/roadmap' as Route}>Roadmap</Link>
          <Link href={'/docs' as Route}>Docs</Link>
          <Link href={'/downloads' as Route}>Emitter</Link>
          <Link href={'/privacy' as Route}>Privacy</Link>
          <Link href={'/terms' as Route}>Terms</Link>
          {/* Brand book §11 names these two OUTBOUND links specifically. They
              were in the flat footers, which the projection hid — so without
              them here they are reachable from nowhere. */}
          <a
            href="https://support.robertsspaceindustries.com/hc/en-us/articles/360006895793"
            target="_blank"
            rel="noopener noreferrer"
          >
            Fandom FAQ
          </a>
          <a
            href="https://robertsspaceindustries.com/en/fankit"
            target="_blank"
            rel="noopener noreferrer"
          >
            RSI Fankit
          </a>
        </>
      }
      disclaimer={
        <>
          Fan-made · Not affiliated with Cloud Imperium Games · RSI · Star
          Citizen™ &amp; Squadron 42™ are trademarks of CIG · Ship, vehicle,
          weapon &amp; item names and specifications © Cloud Imperium Rights
          LLC / Cloud Imperium Rights Ltd — unofficial fan reference; facts
          only, see <Link href="/about#community-data-sources">/about</Link>.{' '}
          {/* READ MORE, on every surface.

              The plate carries the product's attribution verbatim, but it is a
              summary of a longer position: the terms, the privacy statement and
              the trust page are the full text. A trademark and data-source
              notice that cannot be followed to the documents behind it asks a
              reader to take it on faith.

              `/terms` is the entry point rather than a link to each: it carries
              the `LegalIndex`, so Terms, Privacy, Trust and Support are one
              click from here and from each other. */}
          <Link href={'/terms' as Route} className="hp-legal__more">
            Read the full terms &rarr;
          </Link>
        </>
      }
    />
  );
}
