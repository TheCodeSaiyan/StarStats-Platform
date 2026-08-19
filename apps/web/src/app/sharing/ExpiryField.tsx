'use client';

import React, { useEffect, useState } from 'react';

import { utcIsoToLocalInput } from '@/lib/expiry';

/**
 * Timezone-correct auto-expiry picker for the share form.
 *
 * `<input type="datetime-local">` only speaks NAIVE local wall-clock, so a
 * server action can't safely parse or emit it (it would guess the *server's*
 * zone). This component closes the loop from the browser:
 *  - it ships the user's `getTimezoneOffset()` in a hidden `tz_offset_minutes`
 *    field, so the server action converts the submitted wall-clock to a UTC
 *    instant exactly once (`localInputToUtcIso`), and
 *  - it localizes the incoming UTC instant (`prefillIso`) for display, so an
 *    "Edit" round-trip shows the same wall-clock the user originally picked
 *    instead of drifting by the UTC offset each cycle.
 *
 * See `@/lib/expiry` for the pure, unit-tested conversion pair.
 */
export function ExpiryField({
  prefillIso,
  style,
}: {
  prefillIso?: string;
  style?: React.CSSProperties;
}) {
  // The offset is unknown during SSR. Start neutral so the server-rendered
  // markup matches the first client render (no hydration mismatch), then fill
  // in the real offset + localized prefill after mount.
  const [offsetMinutes, setOffsetMinutes] = useState<number | ''>('');
  const [value, setValue] = useState('');

  useEffect(() => {
    const off = new Date().getTimezoneOffset();
    setOffsetMinutes(off);
    if (prefillIso) setValue(utcIsoToLocalInput(prefillIso, off));
  }, [prefillIso]);

  return (
    <>
      <input
        type="hidden"
        name="tz_offset_minutes"
        value={String(offsetMinutes)}
      />
      <input
        type="datetime-local"
        name="expires_at_local"
        value={value}
        onChange={(e) => setValue(e.target.value)}
        aria-label="Auto-expiry (optional)"
        title="Leave blank for no expiry"
        style={style}
      />
    </>
  );
}
