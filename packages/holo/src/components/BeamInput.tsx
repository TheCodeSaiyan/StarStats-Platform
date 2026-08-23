'use client';

import React from 'react';

/**
 * Underline field — the projection has no boxed inputs.
 *
 * The rule is absolute: never a boxed input, never a filled or rounded button.
 * A field is a lit hairline the label sits above.
 */
export interface BeamInputProps
  extends React.InputHTMLAttributes<HTMLInputElement> {
  label?: React.ReactNode;
  /** Sub-line beneath the field — a constraint, a detected default, a caveat. */
  hint?: React.ReactNode;
}

export function BeamInput({
  label,
  hint,
  id,
  className = '',
  ...rest
}: BeamInputProps) {
  const field = (
    <input
      id={id}
      className={['hp-input', className].filter(Boolean).join(' ')}
      {...rest}
    />
  );
  if (!label) return field;
  return (
    <label className="hp-field" htmlFor={id}>
      <span>{label}</span>
      {field}
      {hint ? (
        <span
          style={{
            letterSpacing: '.1em',
            textTransform: 'none',
            opacity: 0.8,
          }}
        >
          {hint}
        </span>
      ) : null}
    </label>
  );
}

/**
 * BeamSelect — the same lit underline, one element down.
 *
 * WHY THIS EXISTS. Select is on the system's deliberately-absent list, and for
 * most of the product that holds: a closed vocabulary of four calibrations is
 * pips, and an on/off is a `BeamSwitch`. But the time-zone control offers ~400
 * IANA zones from `Intl.supportedValuesOf('timeZone')`, and there is no
 * arrangement of pips or switches that is a picker for four hundred things.
 *
 * It stays a NATIVE `<select>` on purpose. A custom listbox would have to
 * re-earn type-to-search, the platform picker on touch, and the whole
 * screen-reader contract — for a control the reader touches roughly once. The
 * open dropdown is therefore OS chrome and not beam, which is a real and
 * accepted limitation: it is true of every native select, and the alternative
 * costs far more than it buys.
 */
export interface BeamSelectProps
  extends React.SelectHTMLAttributes<HTMLSelectElement> {
  label?: React.ReactNode;
  hint?: React.ReactNode;
}

export function BeamSelect({
  label,
  hint,
  id,
  className = '',
  children,
  ...rest
}: BeamSelectProps) {
  const field = (
    <span className="hp-selectwrap">
      <select
        id={id}
        className={['hp-input', 'hp-select', className]
          .filter(Boolean)
          .join(' ')}
        {...rest}
      >
        {children}
      </select>
      {/* The caret is ours — `appearance: none` removes the platform one so the
          control reads as a hairline rather than a box. Geometric, not an icon
          font: there is no icon set in this brand. */}
      <span className="hp-select__caret" aria-hidden="true">
        ▾
      </span>
    </span>
  );
  if (!label) return field;
  return (
    <label className="hp-field" htmlFor={id}>
      <span>{label}</span>
      {field}
      {hint ? (
        <span
          style={{
            letterSpacing: '.1em',
            textTransform: 'none',
            opacity: 0.8,
          }}
        >
          {hint}
        </span>
      ) : null}
    </label>
  );
}

/** Blade switch: a hairline track with a lit blade that slides. */
export function BeamSwitch({
  on = false,
  label,
  description,
  onChange,
  id,
}: {
  on?: boolean;
  label?: React.ReactNode;
  description?: React.ReactNode;
  onChange?: (on: boolean) => void;
  id?: string;
}) {
  return (
    <label className="hp-switch" data-on={on ? 'true' : 'false'} htmlFor={id}>
      <span className="tk" />
      <input
        id={id}
        type="checkbox"
        role="switch"
        checked={on}
        className="sr-only"
        onChange={(e) => onChange && onChange(e.target.checked)}
      />
      <span>
        {label}
        {description ? (
          <span
            style={{
              display: 'block',
              fontSize: 'var(--fs-xs)',
              color: 'var(--dim)',
              letterSpacing: '.08em',
              marginTop: 2,
            }}
          >
            {description}
          </span>
        ) : null}
      </span>
    </label>
  );
}
