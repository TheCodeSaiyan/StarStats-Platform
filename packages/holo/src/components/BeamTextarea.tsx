'use client';

import React from 'react';

/**
 * Multi-line field — `BeamInput`'s sibling, same lit underline.
 *
 * Added to restore a feature the port dropped: the share-report form takes a
 * free-text explanation up to 500 characters, and squeezing that into a
 * single-line input makes a reader compose an incident report through a
 * letterbox. A textarea is a different control, not a bigger input.
 *
 * `resize: vertical` only — horizontal resize would let a reader drag the
 * field out past the pane that contains it.
 */
export interface BeamTextareaProps
  extends React.TextareaHTMLAttributes<HTMLTextAreaElement> {
  label?: React.ReactNode;
  hint?: React.ReactNode;
}

export function BeamTextarea({
  label,
  hint,
  id,
  className = '',
  rows = 3,
  ...rest
}: BeamTextareaProps) {
  const field = (
    <textarea
      id={id}
      rows={rows}
      className={['hp-input', 'hp-textarea', className].filter(Boolean).join(' ')}
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
