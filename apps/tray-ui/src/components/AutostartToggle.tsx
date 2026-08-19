import { useEffect, useState } from 'react';
import { api } from '../api';
import { friendlyError } from '../lib/friendlyError';

/**
 * "Launch StarStats at sign-in" toggle.
 *
 * Reads OS-level state on mount (via `get_autostart_enabled`) rather
 * than the persisted preference — surfaces the ground truth so an
 * external removal (e.g. Task Manager's Startup tab on Windows) is
 * reflected accurately. Writes go through `set_autostart_enabled`,
 * which updates both the OS entry and the persisted preference in
 * one call.
 *
 * Self-contained: no props, no parent state coupling. Drop anywhere
 * the user manages preferences.
 */
export function AutostartToggle() {
  const [enabled, setEnabled] = useState<boolean | null>(null);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;
    api
      .getAutostartEnabled()
      .then((v) => {
        if (!cancelled) setEnabled(v);
      })
      .catch((e) => {
        if (!cancelled) {
          const f = friendlyError(e);
          setError(`${f.title}: ${f.body}`);
        }
      });
    return () => {
      cancelled = true;
    };
  }, []);

  const handleToggle = async (next: boolean) => {
    setSaving(true);
    setError(null);
    try {
      await api.setAutostartEnabled(next);
      setEnabled(next);
    } catch (e) {
      const f = friendlyError(e);
      setError(`${f.title}: ${f.body}`);
    } finally {
      setSaving(false);
    }
  };

  return (
    <label
      style={{
        display: 'flex',
        alignItems: 'flex-start',
        gap: 8,
        fontSize: 12,
        color: 'var(--fg-muted)',
        cursor: enabled === null || saving ? 'default' : 'pointer',
        marginTop: 4,
        borderTop: '1px solid var(--border)',
        paddingTop: 10,
      }}
    >
      <input
        type="checkbox"
        aria-label="Launch StarStats at sign-in"
        checked={enabled === true}
        disabled={enabled === null || saving}
        onChange={(e) => handleToggle(e.target.checked)}
        style={{ accentColor: 'var(--accent)', marginTop: 2 }}
      />
      <span style={{ lineHeight: 1.4 }}>
        <strong style={{ color: 'var(--fg)' }}>Launch at sign-in</strong>
        <span style={{ display: 'block', fontSize: 11 }}>
          Starts StarStats automatically when you sign in. The tray icon
          appears and the main window stays hidden until you click it.
        </span>
        {error && (
          <span
            style={{
              display: 'block',
              marginTop: 4,
              fontSize: 11,
              color: 'var(--danger)',
            }}
          >
            {error}
          </span>
        )}
      </span>
    </label>
  );
}
