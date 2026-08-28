import React from 'react';
import { Projection, Crumb } from 'holo';

/**
 * Projection frame + generic skeleton for a route's pending state.
 *
 * WHY THE FRAME. `loading.tsx` renders with the PAGE absent, and the page is
 * what mounts `.ss-projection-root`. Before this, a fallback drew as a bare
 * skeleton inside `layout.tsx`'s `.ss-app` wearing none of the product's
 * chrome — and, until projection-shell.css stopped keying the shell collapse
 * on `body:has(.ss-projection-root)`, squeezed into the flat era's 220px rail
 * cell with its own scrollbars on an otherwise blank screen.
 *
 * Same shape of fault, and the same fix, as `BoundaryShell` on error and
 * not-found: a render that happens OUTSIDE a page still has to bring the
 * projection with it. The frame is duplicated between the two rather than
 * extracted because they want different columns — a recovery screen reads
 * narrow (`hp-boundary`, 620px), a loading page mirrors the real page's
 * 1040px — and because collapsing them would put an error path and a
 * hot-path fallback in one component.
 *
 * `surface="console"` and `parallax={false}` for the reason BoundaryShell
 * gives: a pending state is not the moment for a parallax stage, and it keeps
 * the fallback cheap on the navigation path.
 *
 * NO `ChromeBar`, deliberately — same argument as BoundaryShell. It needs a
 * session and a nav, which is a data dependency on a screen that exists
 * precisely because data has not arrived.
 *
 * Pass `children` to frame a route's bespoke skeleton; omit them for the
 * generic header + stacked-card rhythm, which mirrors the list/detail pages
 * (/journey, /sharing, /discover, /orgs) so the hand-off on hydration does
 * not shift layout.
 */
interface PageSkeletonProps {
  /** Accessible busy label, also shown as the crumb. e.g. "Loading timeline…". */
  label?: string;
  /** Number of card placeholders in the default skeleton. Ignored with children. */
  cards?: number;
  /** A route-specific skeleton to frame instead of the default. */
  children?: React.ReactNode;
}

export function PageSkeleton({
  label = 'Loading…',
  cards = 4,
  children,
}: PageSkeletonProps) {
  return (
    <div className="ss-projection-root">
      <Projection
        surface="console"
        parallax={false}
        crumb={<Crumb heading parts={[{ t: label }]} />}
      >
        <div className="hp-settings">
          <div
            className="hp-settings__inner"
            aria-busy="true"
            aria-label={label}
          >
            {children ?? (
              <>
                <header>
                  <div
                    className="skeleton"
                    style={{ height: 12, width: 180, marginBottom: 12 }}
                  />
                  <div
                    className="skeleton"
                    style={{ height: 30, width: 260, marginBottom: 10 }}
                  />
                  <div className="skeleton" style={{ height: 14, width: 320 }} />
                </header>
                <div
                  style={{ display: 'flex', flexDirection: 'column', gap: 12 }}
                >
                  {Array.from({ length: cards }).map((_, i) => (
                    <section
                      key={i}
                      className="ss-card"
                      style={{ padding: '18px 20px' }}
                    >
                      <div
                        className="skeleton"
                        style={{ height: 12, width: '40%', marginBottom: 10 }}
                      />
                      <div
                        className="skeleton"
                        style={{ height: 12, width: '72%', marginBottom: 8 }}
                      />
                      <div
                        className="skeleton"
                        style={{ height: 12, width: '55%' }}
                      />
                    </section>
                  ))}
                </div>
              </>
            )}
          </div>
        </div>
      </Projection>
    </div>
  );
}
