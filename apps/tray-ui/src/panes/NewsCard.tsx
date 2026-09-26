/**
 * Staff news in the What's New pane, above the roadmap card.
 *
 * Posts are written in the web admin console. The body is plain text and is
 * rendered as text (`white-space: pre-wrap`), never as HTML, so a post cannot
 * carry markup into the tray. A post is marked read when the player opens it,
 * which clears it from the tab badge on the next poll.
 *
 * Silent when there is nothing to show, including when the tray is unpaired
 * (news read state is per player, so the call needs a token).
 */

import { useCallback, useEffect, useState } from 'react';
import { open as openShell } from '@tauri-apps/plugin-shell';
import { api, type NewsItem } from '../api';
import { TrayCard } from '../components/tray/primitives';
import { relativeTimeSince } from './WhatsNewPane';

export function NewsCard() {
  const [items, setItems] = useState<NewsItem[]>([]);
  const [open, setOpen] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;
    api
      .getNews()
      .then((r) => {
        if (!cancelled) setItems(r.items);
      })
      .catch(() => {
        // Unpaired or offline: the roadmap card below still renders.
      });
    return () => {
      cancelled = true;
    };
  }, []);

  const toggle = useCallback(
    async (item: NewsItem) => {
      setOpen((cur) => (cur === item.id ? null : item.id));
      if (!item.unread) return;
      setItems((prev) => prev.map((i) => (i.id === item.id ? { ...i, unread: false } : i)));
      try {
        await api.markNewsSeen(item.id);
      } catch {
        // The next load shows it unread again; nothing else to do.
      }
    },
    [],
  );

  if (items.length === 0) return null;
  const unread = items.filter((i) => i.unread).length;

  return (
    <TrayCard title="News" kicker={unread > 0 ? `${unread} unread` : 'From the StarStats team'}>
      <ul style={{ listStyle: 'none', margin: 0, padding: 0, display: 'flex', flexDirection: 'column', gap: 8 }}>
        {items.map((item) => {
          const isOpen = open === item.id;
          return (
            <li key={item.id} data-testid="news-item" data-unread={item.unread ? 'true' : undefined}>
              <button
                type="button"
                aria-expanded={isOpen}
                onClick={() => void toggle(item)}
                style={{
                  width: '100%',
                  textAlign: 'left',
                  cursor: 'pointer',
                  background: 'var(--bg-elev)',
                  border: `1px solid ${item.unread ? 'var(--accent)' : 'var(--border)'}`,
                  borderRadius: 'var(--r-md, 8px)',
                  padding: '10px 12px',
                  fontFamily: 'inherit',
                  color: 'var(--fg)',
                  display: 'flex',
                  justifyContent: 'space-between',
                  gap: 8,
                }}
              >
                <span style={{ fontWeight: 600, fontSize: 13 }}>{item.title}</span>
                <span style={{ fontSize: 11, color: 'var(--fg-muted)', whiteSpace: 'nowrap' }}>
                  {relativeTimeSince(item.published_at)}
                </span>
              </button>
              {isOpen ? (
                <div style={{ padding: '8px 12px', fontSize: 13 }}>
                  <div style={{ whiteSpace: 'pre-wrap', lineHeight: 1.5 }}>{item.body}</div>
                  {item.link_url && item.link_url.startsWith('https://') ? (
                    <button
                      type="button"
                      className="tray-btn-ghost"
                      style={{ marginTop: 8 }}
                      onClick={() => void openShell(item.link_url as string).catch(() => {})}
                    >
                      Read more →
                    </button>
                  ) : null}
                </div>
              ) : null}
            </li>
          );
        })}
      </ul>
    </TrayCard>
  );
}
