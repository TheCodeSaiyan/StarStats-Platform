import type { ReactNode } from 'react';
import { NoSignal } from '@/components/hud/NoSignal';
import type { WidgetSize } from './types';

interface Props {
  eyebrow: string;
  title: string;
  size: WidgetSize;
  /** Widget renderer returned null = no data. */
  empty?: boolean;
  children: ReactNode;
}

/**
 * Chrome wrapper for every profile widget. Owns:
 *  - .ss-card section shell
 *  - .ss-eyebrow category label (one per section, above h2)
 *  - h2 title (uses .metric-card__title type tokens for consistency
 *    with the existing metric cards)
 *  - empty-state placeholder when the renderer returns null
 *
 * Does NOT own:
 *  - widget content (passed as children)
 *  - drag handle / visibility eye / size pill (Phase 3)
 *  - data fetching (each widget fetches its own)
 *
 * Styling rule: use existing tokens only. No inline hex colours, no
 * inline pixel paddings that bypass the .ss-card / .metric-card
 * conventions.
 */
export function WidgetFrame({ eyebrow, title, size, empty, children }: Props) {
  return (
    <section
      className="ss-card"
      data-widget-size={size}
      aria-label={title}
    >
      <header className="metric-card__header">
        <div>
          <div className="ss-eyebrow" style={{ marginBottom: 6 }}>
            {eyebrow}
          </div>
          <h2 className="metric-card__title">{title}</h2>
        </div>
      </header>
      <div className="metric-card__body">
        {empty ? <NoSignal compact /> : children}
      </div>
    </section>
  );
}
