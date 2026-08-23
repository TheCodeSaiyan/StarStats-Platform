import React from 'react';

/**
 * Projection action control: a lit hairline box, NEVER a filled pill and never
 * rounded. The equivalent rule for inputs is `BeamInput` — a lit underline,
 * never a boxed field.
 *
 * Forward actions carry a trailing arrow ("Open in catalogue →"). No ellipsis,
 * no exclamation marks, no emoji.
 */
export type BeamButtonVariant = 'default' | 'primary' | 'ghost' | 'danger';

export interface BeamButtonProps
  extends React.ButtonHTMLAttributes<HTMLButtonElement> {
  variant?: BeamButtonVariant;
  /** Render as another element — pass a Next <Link> for real navigation. */
  as?: React.ElementType;
  href?: string;
}

export function BeamButton({
  variant = 'default',
  as = 'button',
  href,
  children,
  className = '',
  ...rest
}: BeamButtonProps) {
  const Tag = (href ? 'a' : as) as React.ElementType;
  const cls = [
    'hp-btn',
    variant !== 'default' ? `hp-btn--${variant}` : '',
    className,
  ]
    .filter(Boolean)
    .join(' ');
  return (
    <Tag className={cls} href={href} {...rest}>
      {children}
    </Tag>
  );
}

/** Status chip — dotted state marker in the beam or a semantic hue. */
export function BeamChip({
  tone,
  dot = false,
  children,
  ...rest
}: {
  tone?: 'warn' | 'good' | 'bad';
  dot?: boolean;
  children?: React.ReactNode;
} & React.HTMLAttributes<HTMLSpanElement>) {
  return (
    <span className={['hp-chip', tone].filter(Boolean).join(' ')} {...rest}>
      {dot ? <i /> : null}
      {children}
    </span>
  );
}
