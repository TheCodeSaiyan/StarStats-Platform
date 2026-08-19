'use client';

import React, { useEffect, useState } from 'react';
import {
  SETTINGS_NAV,
  SETTINGS_NAV_IDS,
} from './settings-nav-config';

/**
 * Client-side scroll-spy sidebar for the Settings two-pane layout (M2).
 *
 * The section content lives in the server `settings/page.tsx` (so the
 * server actions stay in server scope); this component only renders the
 * categorised anchor links and highlights whichever section is currently
 * scrolled into view.
 *
 * Scroll-spy is container-agnostic on purpose: the signed-in shell scrolls
 * inside `.ss-main` (not the window), so we observe intersection against
 * the VIEWPORT (`root: null`). A section physically moves through the
 * viewport as `.ss-main` scrolls, so viewport intersection tracks it
 * correctly regardless of which ancestor owns the scrollbar. The
 * `rootMargin` biases the "active" band to a strip just below the sticky
 * 56px topbar.
 *
 * Clicking a link is a plain in-page anchor jump (`href="#id"`), which
 * also updates the URL fragment — the same fragments the server actions
 * redirect to. We optimistically set the active item on click so the
 * highlight doesn't lag the scroll animation.
 */
export function SettingsNav() {
  const [activeId, setActiveId] = useState<string>(
    SETTINGS_NAV_IDS[0] ?? '',
  );

  useEffect(() => {
    // jsdom / older browsers: no IntersectionObserver → skip scroll-spy
    // (links still work as plain anchors).
    if (typeof IntersectionObserver === 'undefined') return;

    const elements = SETTINGS_NAV_IDS.map((id) =>
      document.getElementById(id),
    ).filter((el): el is HTMLElement => el !== null);
    if (elements.length === 0) return;

    // Track the on-screen top offset of every currently-intersecting
    // section; the active one is whichever sits highest in the band.
    const tops = new Map<string, number>();
    const observer = new IntersectionObserver(
      (entries) => {
        for (const entry of entries) {
          if (entry.isIntersecting) {
            tops.set(entry.target.id, entry.boundingClientRect.top);
          } else {
            tops.delete(entry.target.id);
          }
        }
        let bestId: string | null = null;
        let bestTop = Number.POSITIVE_INFINITY;
        for (const [id, top] of tops) {
          if (top < bestTop) {
            bestTop = top;
            bestId = id;
          }
        }
        // Keep the last active id when nothing is in-band (very top /
        // bottom overscroll) rather than flickering to a default.
        if (bestId) setActiveId(bestId);
      },
      { rootMargin: '-80px 0px -70% 0px', threshold: [0, 1] },
    );

    for (const el of elements) observer.observe(el);
    return () => observer.disconnect();
  }, []);

  return (
    <nav aria-label="Settings" className="ss-settings-nav">
      {SETTINGS_NAV.map((category) => (
        <div key={category.key} className="ss-settings-nav-group">
          <p className="ss-settings-nav-cat">{category.label}</p>
          {category.items.map((item) => {
            const active = item.id === activeId;
            return (
              <a
                key={item.id}
                href={`#${item.id}`}
                className="ss-settings-nav-link"
                aria-current={active ? 'true' : undefined}
                onClick={() => setActiveId(item.id)}
              >
                {item.label}
              </a>
            );
          })}
        </div>
      ))}
    </nav>
  );
}
