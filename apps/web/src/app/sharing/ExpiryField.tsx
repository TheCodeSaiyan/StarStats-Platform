'use client';

import React, { useState } from 'react';

import { utcIsoToLocalInput } from '@/lib/expiry';
import { useIsClient } from '@/lib/use-is-client';

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
  // The offset is unknown during SSR. It stays '' for the server render and
  // the hydration render (so markup matches) and is read from the browser at
  // render time behind the client gate from then on — derived, not copied
  // into state after mount.
  const isClient = useIsClient();
  const offsetMinutes: number | '' = isClient ? new Date().getTimezoneOffset() : '';
  const [value, setValue] = useState('');
  // The localised prefill needs the offset, so it can only be applied on the
  // client. Applied during render and keyed on the prop — React's "adjust
  // state when a prop changes" pattern — so a new prefill on a later render is
  // taken, and a user's own edits in between are not overwritten.
  const [appliedPrefill, setAppliedPrefill] = useState<string | undefined>(undefined);
  if (isClient && prefillIso !== appliedPrefill) {
    setAppliedPrefill(prefillIso);
    if (prefillIso && offsetMinutes !== '') {
      setValue(utcIsoToLocalInput(prefillIso, offsetMinutes));
    }
  }

  return (
    <>
      <input
        type="hidden"
        name="tz_offset_minutes"
        value={String(offsetMinutes)}
      />
      {/* Native `datetime-local`. Like `BeamSelect`, only the CLOSED control
          can be styled — the picker itself is OS chrome — so it takes the
          same `hp-input` lit underline and its own picker-indicator tint. */}
      <input
        type="datetime-local"
        name="expires_at_local"
        className="hp-input hp-datetime"
        value={value}
        onChange={(e) => setValue(e.target.value)}
        aria-label="Auto-expiry (optional)"
        title="Leave blank for no expiry"
        style={style}
      />
    </>
  );
}
