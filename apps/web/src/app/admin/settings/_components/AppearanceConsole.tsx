'use client';

// Classic-JSX-runtime vitest needs the explicit React import.
import React, { useState } from 'react';
import type { AppearanceConfigApi } from '@/lib/api';
import { saveAppearanceConfigAction } from '@/app/_actions/appearance-admin';
import { WAVE_SPEEDS, type WaveSpeed } from '@/lib/wave-speed';

const LABELS: Readonly<Record<WaveSpeed, string>> = {
  off: 'Off',
  slow: 'Slow',
  normal: 'Normal',
  fast: 'Fast',
};

/**
 * Interactive half of /admin/appearance: the sitewide theme-switch
 * wave-speed default. Mirrors `WaitlistConsole`'s client-state +
 * server-action shape — the page itself is a server component; this
 * owns the busy/notice state and the save call.
 */
export function AppearanceConsole({ config }: { config: AppearanceConfigApi }) {
  const [speed, setSpeed] = useState<WaveSpeed>(
    config.theme_wave_speed as WaveSpeed,
  );
  const [busy, setBusy] = useState(false);
  const [notice, setNotice] = useState<string | null>(null);

  async function save(next: WaveSpeed) {
    if (busy || next === speed) return;
    setBusy(true);
    setNotice(null);
    try {
      const res = await saveAppearanceConfigAction({
        theme_wave_speed: next,
      });
      if (!res.ok) {
        setNotice('Save failed — the sitewide default is unchanged.');
        return;
      }
      // Reflect what the server actually stored.
      const stored = res.config.theme_wave_speed as WaveSpeed;
      setSpeed(stored);
      setNotice(
        `Saved. Signed-out visitors and signed-in users without a personal override now get "${LABELS[stored]}".`,
      );
    } finally {
      setBusy(false);
    }
  }

  return (
    <section
      className="ss-card"
      style={{ padding: 'var(--s5) var(--s6)', marginTop: 'var(--s5)' }}
    >
      <div className="ss-placard" style={{ marginBottom: 'var(--s2)' }}>
        Theme-switch wave speed
      </div>
      <p style={{ marginTop: 0, color: 'var(--fg-muted)' }}>
        Sitewide default for the sweep animation duration when a visitor
        switches themes. Applies to signed-out visitors and any
        signed-in user who hasn&apos;t set a personal override in
        Settings.
      </p>
      <div
        role="group"
        aria-label="Sitewide theme-switch wave speed"
        style={{ display: 'flex', gap: 'var(--s3)', flexWrap: 'wrap' }}
      >
        {WAVE_SPEEDS.map((s) => {
          const isActive = s === speed;
          return (
            <button
              key={s}
              type="button"
              className="ss-btn"
              disabled={busy}
              aria-pressed={isActive}
              data-active={isActive ? 'true' : undefined}
              onClick={() => save(s)}
              style={{ fontWeight: isActive ? 600 : 400 }}
            >
              {LABELS[s]}
            </button>
          );
        })}
      </div>
      {notice ? (
        <p role="status" style={{ marginTop: 'var(--s4)' }}>
          {notice}
        </p>
      ) : null}
    </section>
  );
}
