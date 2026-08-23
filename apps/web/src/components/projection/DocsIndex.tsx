import React from 'react';
import Link from 'next/link';
import type { Route } from 'next';

/**
 * The docs index, from `Docs.jsx`.
 *
 * WHAT THE SPEC DOES AND WHY IT MATTERS. The kit's docs surface puts every
 * doc, guide and project page in one grouped row set at the top — Product,
 * Help, Guides, Project — so a reader who lands on "RSI cookie" can see that
 * Troubleshooting and Support exist without going back to a nav. The product
 * splits the same content across eight routes and, before this, each one was a
 * dead end: you arrived, read, and left the way you came.
 *
 * WHERE IT DEPARTS, deliberately. The kit switches `doc` in client state
 * because it is a single mock screen. These are real routes with real URLs,
 * so each entry is a `Link` — shareable, bookmarkable, back-button correct —
 * and the active one is marked with `aria-current` rather than by which
 * `useState` happens to hold.
 *
 * The grouping and its order are the spec's, not a rearrangement.
 */
const GROUPS: readonly [string, readonly [string, string][]][] = [
  [
    'Product',
    [
      ['/features', 'Features'],
      ['/star-platform', 'StarPlatform'],
      ['/about', 'About'],
    ],
  ],
  [
    'Help',
    [
      ['/docs', 'Docs'],
      ['/docs/rsi-cookie', 'RSI cookie'],
      ['/docs/troubleshooting', 'Troubleshooting'],
      ['/support', 'Support'],
    ],
  ],
  [
    'Guides',
    [
      // `/guides` itself is an ADDITION to the spec's group. The kit has no
      // guides landing because it is one screen switching a `doc` id; the
      // product has a real index page and it is in the nav — so an index that
      // omitted the page a reader was standing on would be the one link they
      // could not find.
      ['/guides', 'All guides'],
      ['/guides/dashboard', 'Dashboard'],
      ['/guides/desktop-app', 'Desktop app'],
      ['/guides/settings', 'Settings'],
      ['/guides/sharing', 'Sharing'],
    ],
  ],
  [
    'Project',
    [
      ['/changelog', 'Changelog'],
      ['/roadmap', 'Roadmap'],
      ['/lore', 'Lore'],
    ],
  ],
];

export function DocsIndex({ active }: { active: string }) {
  return (
    <nav className="hp-docsindex" aria-label="Documentation">
      {GROUPS.map(([group, items]) => (
        <div className="hp-docsindex__row" key={group}>
          <span className="hp-docsindex__grp">{group}</span>
          {items.map(([href, label]) => (
            <Link
              key={href}
              href={href as Route}
              className="hp-docsindex__lk"
              aria-current={href === active ? 'page' : undefined}
            >
              {label}
            </Link>
          ))}
        </div>
      ))}
    </nav>
  );
}
