'use client';

import React from 'react';
import { CALIBRATIONS, type CalibrationId } from './ChromeBar';

/**
 * The verbose calibration picker — the settings counterpart to
 * `CalibrationPips`.
 *
 * Pips are right for the chrome, where the control is one of a dozen things
 * competing for a row. On a settings surface the reader is deciding, so each
 * calibration gets its name and its character.
 *
 * PROGRESSIVE ENHANCEMENT, and it is the reason this is a `<form>` rather than
 * a row of buttons: each swatch is a submit button carrying its own value, so
 * with JavaScript off a click posts to `formAction` and the page reloads with
 * the new beam. With JavaScript on, `onSelect` intercepts and the change
 * happens in place. A preference control that stops working when a bundle
 * fails is a poor preference control.
 *
 * RECALIBRATION IS AN EVENT, NOT A REPAINT. `onSelect` is expected to bump the
 * consumer's `recalKey` as well as persist, so the shock ring, scan wipe and
 * emitter surge fire. Repainting the tokens silently would throw away the one
 * piece of orchestrated motion the system has.
 */
const CHARACTER: Record<CalibrationId, string> = {
  terra: 'Cyan · clinical',
  stanton: 'Amber · warm',
  pyro: 'Coral · aggressive',
  nyx: 'Violet · cold',
};

export interface CalibrationChoiceProps {
  active: string;
  /**
   * Server action the form posts to when JavaScript is unavailable. Each
   * button submits `name="theme"` with its calibration id.
   */
  formAction?: (formData: FormData) => void | Promise<void>;
  /** Client handler. When present it intercepts the submit. */
  onSelect?: (id: CalibrationId) => void;
  /** Form field name. Defaults to `theme`, which is what the API stores. */
  name?: string;
}

export function CalibrationChoice({
  active,
  formAction,
  onSelect,
  name = 'theme',
}: CalibrationChoiceProps) {
  return (
    <form action={formAction} style={{ margin: 0 }}>
      <div className="hp-calchoice">
        {CALIBRATIONS.map((c) => {
          const isActive = c.id === active;
          return (
            <button
              key={c.id}
              type="submit"
              name={name}
              value={c.id}
              aria-pressed={isActive}
              aria-label={`${c.name} calibration`}
              data-active={isActive ? 'true' : undefined}
              style={{ ['--pip' as string]: c.pip } as React.CSSProperties}
              onClick={
                onSelect
                  ? (e) => {
                      e.preventDefault();
                      onSelect(c.id);
                    }
                  : undefined
              }
            >
              <span className="pip" aria-hidden="true" />
              <span className="nm">{c.name}</span>
              <span className="ch">{CHARACTER[c.id]}</span>
            </button>
          );
        })}
      </div>
    </form>
  );
}

/**
 * A closed set of values as lit underlines — the same treatment `RangeTabs`
 * uses, generalised for any short vocabulary posted through a form.
 *
 * Same progressive-enhancement contract as `CalibrationChoice`: submits
 * without JavaScript, intercepts with it.
 */
export interface BeamChoiceProps {
  name: string;
  value: string;
  options: readonly { value: string; label?: React.ReactNode }[];
  formAction?: (formData: FormData) => void | Promise<void>;
  onSelect?: (value: string) => void;
  'aria-label'?: string;
}

export function BeamChoice({
  name,
  value,
  options,
  formAction,
  onSelect,
  'aria-label': ariaLabel,
}: BeamChoiceProps) {
  return (
    <form action={formAction} style={{ margin: 0 }}>
      <div className="hp-rng" style={{ marginLeft: 0 }} aria-label={ariaLabel}>
        {options.map((o) => (
          <button
            key={o.value}
            type="submit"
            name={name}
            value={o.value}
            aria-pressed={o.value === value}
            onClick={
              onSelect
                ? (e) => {
                    e.preventDefault();
                    onSelect(o.value);
                  }
                : undefined
            }
          >
            {o.label ?? o.value}
          </button>
        ))}
      </div>
    </form>
  );
}
