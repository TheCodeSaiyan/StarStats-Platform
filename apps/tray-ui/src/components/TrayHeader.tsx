/**
 * Tray header — mirrors `tray-app.jsx`'s `TrayHeader`. 3-col grid:
 * brand mark + version on the left, view tabs in the centre, tailing
 * status pill on the right.
 */

import { StatusDot } from './tray/primitives';

export type TrayView =
  | 'status'
  | 'logs'
  | 'kb'
  | 'whats-new'
  | 'review'
  | 'settings';

interface Props {
  view: TrayView;
  onView: (next: TrayView) => void;
  isTailing: boolean;
  /**
   * Cargo workspace version of the running binary. `null` while the
   * IPC fetch is in flight — render the brand mark without a
   * trailing version string in that case rather than flashing a
   * stale fallback.
   */
  version: string | null;
  /** Number of unknown shapes pending review. Shown as a small
   *  badge on the Review tab. 0 hides the badge. */
  reviewBadge?: number;
}

const TABS: ReadonlyArray<TrayView> = [
  'status',
  'logs',
  'kb',
  'whats-new',
  'review',
  'settings',
];

/// Brand book §02 v2: display labels are decoupled from the TrayView
/// route keys so the identifiers stay stable as code anchors.
const TAB_LABELS: Record<TrayView, string> = {
  status: 'Readout',
  logs: 'Manifest',
  kb: 'Catalogue',
  'whats-new': "What's New",
  review: 'Review',
  settings: 'Calibrate',
};

/// Plain-language function names surfaced as a hover `title` so the
/// brand-flavored tab labels stay discoverable (a new user can't guess
/// "Calibrate" = Settings). These also align with the web app's plain
/// nomenclature without overriding the brand-book display labels above.
const TAB_TITLES: Record<TrayView, string> = {
  status: 'Status',
  logs: 'Logs',
  kb: 'Knowledge base',
  'whats-new': "What's New",
  review: 'Review',
  settings: 'Settings',
};

export function TrayHeader({ view, onView, isTailing, version, reviewBadge = 0 }: Props) {
  return (
    <header
      // Tone + spacing aligned to the web `.ss-topbar` (bg + --s4 gap,
      // starstats-tokens.css) — the horizontal-tab layout stays tray-only
      // (M4; form-factor is intentional, not a re-skin to a vertical rail).
      style={{
        display: 'grid',
        gridTemplateColumns: 'auto 1fr auto',
        alignItems: 'center',
        gap: 'var(--s4)',
        padding: '12px 16px',
        borderBottom: '1px solid var(--border)',
        background: 'var(--bg)',
      }}
    >
      <div style={{ display: 'flex', alignItems: 'center', gap: 10 }}>
        <span
          style={{
            fontFamily: 'var(--font-mono)',
            fontSize: 16,
            color: 'var(--accent)',
            fontWeight: 700,
            letterSpacing: '-0.02em',
          }}
          aria-hidden="true"
        >
          ★
        </span>
        <div style={{ display: 'flex', flexDirection: 'column' }}>
          <div
            style={{
              fontWeight: 700,
              fontSize: 13,
              letterSpacing: '0.06em',
              textTransform: 'uppercase',
            }}
          >
            STARSTATS
          </div>
          <div style={{ fontSize: 10, color: 'var(--fg-dim)', letterSpacing: '0.04em' }}>
            {version ? `Uplink · v${version}` : 'Uplink'}
          </div>
        </div>
      </div>

      <nav style={{ display: 'flex', gap: 4, justifyContent: 'center' }} aria-label="Pane">
        {TABS.map((tab) => {
          const active = view === tab;
          const showBadge = tab === 'review' && reviewBadge > 0;
          return (
            <button
              key={tab}
              type="button"
              onClick={() => onView(tab)}
              // Expose the active view to AT — previously the selected
              // tab was distinguished by colour/border only (L7).
              aria-current={active ? 'page' : undefined}
              title={TAB_TITLES[tab]}
              aria-label={
                showBadge
                  ? `Review, ${reviewBadge} unknown ${reviewBadge === 1 ? 'line' : 'lines'}`
                  : undefined
              }
              style={{
                background: active ? 'var(--accent-soft)' : 'transparent',
                color: active ? 'var(--accent)' : 'var(--fg-muted)',
                border: `1px solid ${active ? 'var(--accent)' : 'transparent'}`,
                borderRadius: 'var(--r-sm)',
                padding: '5px 14px',
                fontFamily: 'inherit',
                fontSize: 12,
                fontWeight: 600,
                textTransform: 'uppercase',
                letterSpacing: '0.08em',
                cursor: 'pointer',
                display: 'inline-flex',
                alignItems: 'center',
                gap: 6,
              }}
            >
              {TAB_LABELS[tab]}
              {showBadge && (
                <span
                  data-testid="review-badge"
                  style={{
                    background: 'var(--accent)',
                    color: 'var(--bg)',
                    borderRadius: 'var(--r-pill, 999px)',
                    fontSize: 10,
                    lineHeight: 1,
                    padding: '2px 6px',
                    fontWeight: 700,
                  }}
                >
                  {reviewBadge}
                </span>
              )}
            </button>
          );
        })}
      </nav>

      <div
        style={{
          display: 'flex',
          alignItems: 'center',
          gap: 6,
          fontSize: 11,
          color: 'var(--fg-muted)',
        }}
      >
        <StatusDot tone={isTailing ? 'ok' : 'dim'} />
        <span style={{ fontFamily: 'var(--font-mono)' }}>
          {isTailing ? 'TAILING' : 'IDLE'}
        </span>
      </div>
    </header>
  );
}
