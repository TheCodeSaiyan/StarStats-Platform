'use client';

import React from 'react';
import { ConfirmSubmitButton } from '@/components/forms/ConfirmSubmitButton';

/**
 * Time-zone picker for the preferences card.
 *
 * A client component because the useful default is the browser's own zone —
 * `Intl.DateTimeFormat().resolvedOptions().timeZone` — which the server
 * cannot know. Nothing is saved without the player pressing Save: detecting
 * a value and silently writing it would be a mutation triggered by a page
 * view.
 *
 * The option list comes from `Intl.supportedValuesOf('timeZone')` where
 * available, so it always matches the browser's own tz database rather than
 * a list that rots. Older browsers fall back to whatever is already stored
 * plus the detected zone, which is enough to set it correctly.
 */
export function TimezoneControl({
  storedTimezone,
  timezoneAction,
}: {
  storedTimezone: string | null;
  timezoneAction: (formData: FormData) => Promise<void>;
}) {
  const detected = React.useMemo(() => {
    try {
      return Intl.DateTimeFormat().resolvedOptions().timeZone || null;
    } catch {
      return null;
    }
  }, []);

  const zones = React.useMemo(() => {
    const withSupported = (
      Intl as unknown as { supportedValuesOf?: (k: string) => string[] }
    ).supportedValuesOf;
    let list: string[] = [];
    if (typeof withSupported === 'function') {
      try {
        list = withSupported('timeZone');
      } catch {
        list = [];
      }
    }
    // Guarantee the stored and detected zones are selectable even when the
    // browser gives us no list.
    const extras = [storedTimezone, detected].filter(
      (z): z is string => !!z && !list.includes(z),
    );
    return [...extras, ...list];
  }, [storedTimezone, detected]);

  const initial = storedTimezone ?? detected ?? '';

  return (
    <form action={timezoneAction} style={{ display: 'flex', gap: 8, flexWrap: 'wrap' }}>
      <label className="sr-only" htmlFor="timezone-select">
        Time zone
      </label>
      <select
        id="timezone-select"
        name="timezone"
        defaultValue={initial}
        className="ss-input"
        style={{ minWidth: 220 }}
      >
        {!initial && <option value="">Select a time zone…</option>}
        {zones.map((z) => (
          <option key={z} value={z}>
            {z.replace(/_/g, ' ')}
          </option>
        ))}
      </select>
      <ConfirmSubmitButton className="ss-btn">Save</ConfirmSubmitButton>
      {!storedTimezone && detected && (
        <p style={{ width: '100%', margin: 0, color: 'var(--fg-muted)', fontSize: 12 }}>
          Detected {detected.replace(/_/g, ' ')} from your browser.
        </p>
      )}
    </form>
  );
}
