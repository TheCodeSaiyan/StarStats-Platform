/**
 * Tray release notes in What's New: the newest release on the player's
 * channel open, older ones folded to one line. Notes are generated from
 * commit subjects by scripts/release-notes.mjs and grouped New / Improved /
 * Fixed; the card renders them as text.
 *
 * Opening a release marks it read, which clears it from the tab badge on
 * the next poll. The newest is open by default, so it counts as read once
 * the player has looked at the pane.
 *
 * Silent when there is nothing to show, including when unpaired.
 */

import { useEffect, useState } from 'react';
import { api, type ReleaseItem } from '../api';
import { TrayCard } from '../components/tray/primitives';

const KIND_COLOUR: Record<string, string> = {
  New: 'var(--ok, #74C68A)',
  Improved: 'var(--accent)',
  Fixed: '#8FB4E8',
};

export function ReleasesCard() {
  const [items, setItems] = useState<ReleaseItem[]>([]);
  const [open, setOpen] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;
    api
      .getReleases()
      .then((r) => {
        if (cancelled) return;
        setItems(r.releases);
        const newest = r.releases[0];
        if (newest) {
          setOpen(newest.id);
          if (newest.unread) void markRead(newest.id);
        }
      })
      .catch(() => {
        // Unpaired or offline: the other cards still render.
      });
    return () => {
      cancelled = true;
    };
  }, []);

  async function markRead(id: string) {
    setItems((prev) => prev.map((r) => (r.id === id ? { ...r, unread: false } : r)));
    try {
      await api.markReleaseSeen(id);
    } catch {
      // Shown unread again on the next load; nothing else to do.
    }
  }

  const toggle = (r: ReleaseItem) => {
    setOpen((cur) => (cur === r.id ? null : r.id));
    if (r.unread) void markRead(r.id);
  };

  if (items.length === 0) return null;

  return (
    <TrayCard title="Releases" kicker="Tray updates">
      <ul style={{ listStyle: 'none', margin: 0, padding: 0, display: 'flex', flexDirection: 'column', gap: 8 }}>
        {items.map((r) => {
          const isOpen = open === r.id;
          return (
            <li key={r.id} data-testid="release-item" data-unread={r.unread ? 'true' : undefined}>
              <button
                type="button"
                aria-expanded={isOpen}
                onClick={() => toggle(r)}
                style={{
                  width: '100%',
                  textAlign: 'left',
                  cursor: 'pointer',
                  background: 'var(--bg-elev)',
                  border: `1px solid ${r.unread ? 'var(--accent)' : 'var(--border)'}`,
                  borderRadius: 'var(--r-md, 8px)',
                  padding: '10px 12px',
                  fontFamily: 'inherit',
                  color: 'var(--fg)',
                  display: 'flex',
                  justifyContent: 'space-between',
                  gap: 8,
                }}
              >
                <span style={{ fontWeight: 600, fontSize: 13 }}>
                  StarStats {r.version}
                  {r.channel !== 'live' ? (
                    <span style={{ color: 'var(--accent)', fontSize: 10, marginLeft: 6, textTransform: 'uppercase' }}>
                      {r.channel}
                    </span>
                  ) : null}
                </span>
                <span style={{ fontSize: 11, color: 'var(--fg-muted)', whiteSpace: 'nowrap' }}>
                  {r.summary || 'No player-facing changes'} · {r.released_on}
                </span>
              </button>
              {isOpen ? (
                <div style={{ padding: '8px 12px', display: 'flex', flexDirection: 'column', gap: 8 }}>
                  {r.notes.length === 0 ? (
                    <span style={{ color: 'var(--fg-muted)', fontSize: 12 }}>
                      Behind-the-scenes changes only.
                    </span>
                  ) : (
                    r.notes.map((g) => (
                      <div key={g.kind}>
                        <div
                          style={{
                            fontSize: 10,
                            fontWeight: 600,
                            letterSpacing: '0.1em',
                            textTransform: 'uppercase',
                            color: KIND_COLOUR[g.kind] ?? 'var(--fg-muted)',
                          }}
                        >
                          {g.kind}
                        </div>
                        <ul style={{ margin: '4px 0 0', paddingLeft: 16, fontSize: 12.5, lineHeight: 1.45 }}>
                          {g.lines.map((l) => (
                            <li key={l.text}>{l.text}</li>
                          ))}
                        </ul>
                      </div>
                    ))
                  )}
                </div>
              ) : null}
            </li>
          );
        })}
      </ul>
    </TrayCard>
  );
}
