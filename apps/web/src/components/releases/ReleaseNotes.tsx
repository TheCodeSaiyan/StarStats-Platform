import React from 'react';
import type { NoteGroup } from '@/lib/releases';

const KIND_COLOUR: Record<string, string> = {
  New: 'var(--ok, #74C68A)',
  Improved: 'var(--accent)',
  Fixed: 'var(--info, #8FB4E8)',
};

/**
 * One release's notes, grouped New / Improved / Fixed, each line tagged
 * with where it shows up (Tray, Web). Text only: lines come from commit
 * subjects and are never rendered as HTML.
 */
export function ReleaseNotes({ groups }: { groups: readonly NoteGroup[] }) {
  if (groups.length === 0) {
    return <p style={{ color: 'var(--fg-dim)', margin: 0 }}>Behind-the-scenes changes only.</p>;
  }
  return (
    <div style={{ display: 'flex', flexDirection: 'column', gap: 12 }}>
      {groups.map((g) => (
        <div key={g.kind} data-testid={`release-group-${g.kind}`}>
          <h4
            style={{
              margin: '0 0 6px',
              fontSize: 11,
              fontWeight: 600,
              letterSpacing: '0.1em',
              textTransform: 'uppercase',
              color: KIND_COLOUR[g.kind] ?? 'var(--fg-dim)',
            }}
          >
            {g.kind}
          </h4>
          <ul style={{ margin: 0, paddingLeft: 18, display: 'flex', flexDirection: 'column', gap: 4 }}>
            {g.lines.map((l) => (
              <li key={l.text} style={{ lineHeight: 1.5 }}>
                {l.text}
                {l.surfaces.map((s) => (
                  <span
                    key={s}
                    style={{
                      marginLeft: 6,
                      fontSize: 10,
                      padding: '1px 5px',
                      border: '1px solid var(--border)',
                      color: 'var(--fg-dim)',
                      whiteSpace: 'nowrap',
                    }}
                  >
                    {s}
                  </span>
                ))}
              </li>
            ))}
          </ul>
        </div>
      ))}
    </div>
  );
}
