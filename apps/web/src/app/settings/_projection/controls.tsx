'use client';

import React from 'react';
import { BeamSelect, BeamButton, BeamChoice } from 'holo';
import { WAVE_SPEEDS, type WaveSpeed } from '@/lib/wave-speed';

/**
 * Projection-native replacements for the three flat settings controls.
 *
 * All three keep their EXISTING server action and form contract exactly —
 * same field names (`theme`, `wave_speed`, `timezone`), same no-JS submit
 * path. Only the drawing changed.
 */

/**
 * Wave speed. A closed set of four, so it takes `BeamChoice` — the system's
 * lit-underline treatment for a short vocabulary — rather than becoming a
 * fifth control shape.
 *
 * No `onSelect`: unlike calibration, wave speed has nothing to preview in
 * place, so the plain form submit is the whole behaviour.
 */
export function WaveSpeedField({
  active,
  waveSpeedAction,
}: {
  active: WaveSpeed;
  waveSpeedAction: (formData: FormData) => void | Promise<void>;
}) {
  return (
    <BeamChoice
      name="wave_speed"
      value={active}
      aria-label="Wave speed"
      options={WAVE_SPEEDS.map((s) => ({ value: s }))}
      formAction={waveSpeedAction}
    />
  );
}

/**
 * Time zone.
 *
 * Client-side because the useful default is the browser's own zone, which the
 * server cannot know — and nothing is saved without the reader pressing Save:
 * detecting a value and silently writing it would be a mutation triggered by a
 * page view. The option list comes from the browser's own tz database so it
 * cannot rot against a hardcoded list.
 *
 * Logic lifted verbatim from `TimezoneControl`; only the field is new.
 */
export function TimezoneField({
  storedTimezone,
  timezoneAction,
}: {
  storedTimezone: string | null;
  timezoneAction: (formData: FormData) => void | Promise<void>;
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
    <form action={timezoneAction} className="hp-formrow">
      <BeamSelect
        id="timezone-select"
        name="timezone"
        label="Time zone"
        defaultValue={initial}
        hint={
          !storedTimezone && detected
            ? `Detected ${detected.replace(/_/g, ' ')} from your browser.`
            : undefined
        }
      >
        {!initial && <option value="">Select a time zone…</option>}
        {zones.map((z) => (
          <option key={z} value={z}>
            {z.replace(/_/g, ' ')}
          </option>
        ))}
      </BeamSelect>
      <BeamButton type="submit">Save</BeamButton>
    </form>
  );
}
