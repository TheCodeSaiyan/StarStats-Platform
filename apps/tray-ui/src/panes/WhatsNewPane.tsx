/**
 * Tray UI — "What's new" panel (Phase 8, roadmap spec §9).
 *
 * Caps at 3 unread top-level roadmap items (or the 3 most-recently
 * shipped items for the anonymous tray). Each card surfaces the
 * title, a headline-status chip, and the time since the latest
 * published changelog entry. Clicking a card marks-seen via the
 * Tauri command and opens the public `/roadmap/{slug}` page.
 *
 * Data path goes Rust-side via two Tauri commands so the WebView's
 * CSP doesn't block the cross-origin HTTP (same pattern as the
 * Catalogue pane's `get_reference_category`).
 *
 * There are no `position: fixed` descendants here — the panel is a
 * pure list — so the App.tsx `ss-screen-enter` transform-trap rule
 * (portal fixed elements to `document.body`) is moot for this pane.
 */

import { useCallback, useEffect, useState } from 'react';
import { open as openShell } from '@tauri-apps/plugin-shell';
import { api, type WhatsNewItem } from '../api';
import { GhostButton, TrayCard } from '../components/tray/primitives';

// `WhatsNewItem` / `WhatsNewResponse` are defined once on the api
// surface (`../api`). Re-exported here so existing importers (the
// pane's test) keep resolving them from this module.
export type { WhatsNewItem, WhatsNewResponse } from '../api';

interface Props {
  /** Public web origin (e.g. `https://starstats.app`). When `null`
   *  the tray hasn't yet resolved one — link-out buttons stay
   *  disabled rather than navigating to `null/roadmap/...`. */
  webOrigin: string | null;
}

/// Maximum cards the panel shows. Spec §9: "caps at 3 unread top-level
/// items." The server also enforces this cap, but defensive UI-side
/// trimming keeps the panel honest if the server ever loosens it.
const MAX_CARDS = 3;

/// Map server-side status strings to a tone for the headline chip.
/// Mirrors the rough palette used by the web roadmap card; kept inline
/// because the tray's design tokens already cover these tones.
function toneForStatus(status: string): { bg: string; fg: string } {
  switch (status) {
    case 'shipped':
      return { bg: 'var(--ok-soft, #1f3a2a)', fg: 'var(--ok, #6cc09a)' };
    case 'beta':
      return { bg: 'var(--warn-soft, #3a2f1f)', fg: 'var(--warn, #d9a05a)' };
    case 'building':
    case 'in-design':
      return { bg: 'var(--accent-soft)', fg: 'var(--accent)' };
    case 'parked':
      return { bg: 'var(--bg-elev)', fg: 'var(--fg-dim)' };
    default:
      return { bg: 'var(--bg-elev)', fg: 'var(--fg-muted)' };
  }
}

/// Relative-time formatter used on each card ("3h ago", "yesterday",
/// "12d ago"). Cheap inline impl — no dep on a date-fns equivalent.
export function relativeTimeSince(iso: string, now: Date = new Date()): string {
  const then = new Date(iso);
  const ms = now.getTime() - then.getTime();
  if (Number.isNaN(ms)) return '';
  const sec = Math.max(0, Math.floor(ms / 1000));
  if (sec < 60) return 'just now';
  const min = Math.floor(sec / 60);
  if (min < 60) return `${min}m ago`;
  const hr = Math.floor(min / 60);
  if (hr < 24) return `${hr}h ago`;
  const day = Math.floor(hr / 24);
  if (day < 7) return `${day}d ago`;
  const wk = Math.floor(day / 7);
  if (wk < 5) return `${wk}w ago`;
  const mo = Math.floor(day / 30);
  if (mo < 12) return `${mo}mo ago`;
  const yr = Math.floor(day / 365);
  return `${yr}y ago`;
}

export function WhatsNewPane({ webOrigin }: Props) {
  const [items, setItems] = useState<WhatsNewItem[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [seenViaAuth, setSeenViaAuth] = useState(false);

  const refresh = useCallback(async () => {
    setLoading(true);
    setError(null);
    try {
      const resp = await api.getWhatsNew();
      setItems(resp.items.slice(0, MAX_CARDS));
      setSeenViaAuth(resp.seen_via_auth);
    } catch (e) {
      setError(String(e));
      setItems([]);
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    void refresh();
  }, [refresh]);

  const handleCardClick = useCallback(
    async (item: WhatsNewItem) => {
      // Best-effort mark-seen — even if the IPC call fails, we still
      // navigate so the user isn't blocked. The next refresh will
      // re-classify the item as unread and the panel will try again.
      if (seenViaAuth) {
        try {
          await api.markWhatsNewSeen(
            item.roadmap_item_id,
            item.latest_changelog_entry_id,
          );
        } catch {
          // Silent — see comment above.
        }
      }
      if (webOrigin) {
        try {
          await openShell(`${webOrigin}/roadmap/${item.slug}`);
        } catch {
          // openShell can reject if the user's OS lacks a default
          // browser handler. The card has already been marked seen
          // server-side at this point — accept the no-op.
        }
      }
      // Drop the seen card locally so the panel feels responsive
      // before the next /whats-new round-trip.
      setItems((prev) => prev.filter((i) => i.roadmap_item_id !== item.roadmap_item_id));
    },
    [seenViaAuth, webOrigin],
  );

  const handleMoreOnWeb = useCallback(async () => {
    if (!webOrigin) return;
    try {
      await openShell(`${webOrigin}/roadmap`);
    } catch {
      // See handleCardClick.
    }
  }, [webOrigin]);

  if (loading) {
    return (
      <div className="loading" role="status">
        Loading…
      </div>
    );
  }

  if (error) {
    return (
      <TrayCard title="What's new" kicker="Roadmap">
        <div style={{ color: 'var(--danger)', fontSize: 12 }}>{error}</div>
      </TrayCard>
    );
  }

  return (
    <TrayCard
      title="What's new"
      kicker={seenViaAuth ? 'Roadmap · unread' : 'Roadmap · recent'}
    >
      {items.length === 0 ? (
        <div
          data-testid="whatsnew-empty"
          style={{
            color: 'var(--fg-muted)',
            fontSize: 13,
            padding: '8px 0',
          }}
        >
          All caught up — nothing new.
        </div>
      ) : (
        <ul
          style={{
            listStyle: 'none',
            margin: 0,
            padding: 0,
            display: 'flex',
            flexDirection: 'column',
            gap: 8,
          }}
        >
          {items.map((item) => {
            const tone = toneForStatus(item.headline_status);
            return (
              <li key={item.roadmap_item_id}>
                <button
                  type="button"
                  onClick={() => void handleCardClick(item)}
                  data-testid="whatsnew-item"
                  data-slug={item.slug}
                  style={{
                    width: '100%',
                    textAlign: 'left',
                    cursor: 'pointer',
                    background: 'var(--bg-elev)',
                    border: '1px solid var(--border)',
                    borderRadius: 'var(--r-md, 8px)',
                    padding: '10px 12px',
                    display: 'flex',
                    flexDirection: 'column',
                    gap: 6,
                    fontFamily: 'inherit',
                    color: 'var(--fg)',
                  }}
                >
                  <div
                    style={{
                      display: 'flex',
                      alignItems: 'center',
                      justifyContent: 'space-between',
                      gap: 8,
                    }}
                  >
                    <span style={{ fontWeight: 600, fontSize: 13 }}>{item.title}</span>
                    <span
                      style={{
                        background: tone.bg,
                        color: tone.fg,
                        fontSize: 10,
                        fontWeight: 700,
                        textTransform: 'uppercase',
                        letterSpacing: '0.06em',
                        padding: '2px 6px',
                        borderRadius: 'var(--r-pill, 999px)',
                      }}
                    >
                      {item.headline_status}
                    </span>
                  </div>
                  <div style={{ fontSize: 11, color: 'var(--fg-muted)' }}>
                    {relativeTimeSince(item.latest_published_at)}
                  </div>
                </button>
              </li>
            );
          })}
        </ul>
      )}
      <div style={{ marginTop: 12, display: 'flex', justifyContent: 'flex-end' }}>
        <GhostButton
          onClick={() => void handleMoreOnWeb()}
          disabled={!webOrigin}
        >
          More on web →
        </GhostButton>
      </div>
    </TrayCard>
  );
}
