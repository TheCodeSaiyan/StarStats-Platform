/**
 * Tray-specific primitives mirroring the design package's
 * `tray-app.jsx` atoms. Compact density variants of the web's `ss-*`
 * primitives — tray runs at ~720px, so paddings and font sizes are
 * tighter than the equivalent web-app cards.
 *
 * Consumers: StatusPane, SettingsPane, future LogsPane.
 */

import { type ReactNode, type CSSProperties, type InputHTMLAttributes, type ButtonHTMLAttributes } from 'react';

export type Tone = 'ok' | 'warn' | 'danger' | 'accent' | 'info' | 'dim' | 'default';

interface TrayCardProps {
  title?: ReactNode;
  kicker?: ReactNode;
  right?: ReactNode;
  children: ReactNode;
  mono?: boolean;
}

export function TrayCard({ title, kicker, right, children, mono = false }: TrayCardProps) {
  // The `ss-card` class wires this surface into the design system's
  // animations (mount-in, hover lift) and the `.ss-screen-enter`
  // nth-child stagger. We keep the tray-specific tight padding inline
  // — the global `.ss-card-pad` is the wider web-app density.
  return (
    <section
      className="ss-card"
      style={{
        padding: '14px 16px',
      }}
    >
      {(title || right) && (
        <header
          style={{
            display: 'flex',
            alignItems: 'baseline',
            justifyContent: 'space-between',
            gap: 12,
            marginBottom: 10,
          }}
        >
          <div style={{ display: 'flex', alignItems: 'baseline', gap: 8 }}>
            {title && (
              // `.hud-tile__title` wires this into the shared HUD instrument
              // typography (and the shared mobile h1/h2 shrink rule's
              // `:not(.hud-tile__title)` exclusion — see starstats-tokens.css).
              // The inline overrides below keep TrayCard's compact density
              // (11px/muted) distinct from the web tile's larger plain bold
              // title; they win over the class on every property they set,
              // so this is additive wiring, not a re-skin. No small-caps
              // treatment here — TrayCard renders many times per pane, so an
              // eyebrow-style title would repeat the eyebrow look on every
              // card, which is outside the two sanctioned `.ss-eyebrow` uses
              // (L8 ruling). Plain label instead (M4).
              <h2
                className="hud-tile__title"
                style={{
                  margin: 0,
                  fontSize: 11,
                  fontWeight: 600,
                  color: 'var(--fg-muted)',
                  fontFamily: mono ? 'var(--font-mono)' : 'var(--font-sans)',
                }}
              >
                {title}
              </h2>
            )}
            {kicker && (
              <span
                style={{
                  fontSize: 11,
                  color: 'var(--fg-dim)',
                  fontFamily: 'var(--font-mono)',
                }}
              >
                {kicker}
              </span>
            )}
          </div>
          {right}
        </header>
      )}
      {children}
    </section>
  );
}

interface KVProps {
  label: ReactNode;
  value: ReactNode;
  mono?: boolean;
  dim?: boolean;
}

export function KV({ label, value, mono = false, dim = false }: KVProps) {
  return (
    <>
      {/* `.k` only picks up its 9px/uppercase/dim treatment nested inside
          `.hud-readout` (see hud.css) — same nesting the web widgets use
          for inline label+value pairs. Here dt/dd are siblings (the
          call sites lay them out via a `<dl>` CSS grid), so the label
          gets its own `.hud-readout` wrapper to opt into that rule. */}
      <dt className="hud-readout">
        <span className="k">{label}</span>
      </dt>
      <dd
        className="hud-readout"
        style={{
          margin: 0,
          fontVariantNumeric: 'tabular-nums',
          fontSize: 13,
          color: dim ? 'var(--fg-dim)' : 'var(--fg)',
          fontFamily: mono ? 'var(--font-mono)' : 'var(--font-sans)',
          wordBreak: mono ? 'break-all' : 'normal',
        }}
      >
        {value}
      </dd>
    </>
  );
}

interface StatPillProps {
  label: ReactNode;
  value: ReactNode;
  tone?: Tone;
}

const PILL_TONES: Record<Tone, string> = {
  default: 'var(--fg)',
  ok: 'var(--ok)',
  warn: 'var(--warn)',
  danger: 'var(--danger)',
  accent: 'var(--accent)',
  info: 'var(--info)',
  dim: 'var(--fg-dim)',
};

export function StatPill({ label, value, tone = 'default' }: StatPillProps) {
  // `.hud-tile` supplies the shared HUD tile chrome (bg-elev surface,
  // border, radius, and — via `--hud-pad` — a 7px/9px pad that's
  // already tighter than this pill's previous 8px/10px, so the tray's
  // denser-than-web density falls out of the shared class for free).
  // Label + value reuse `.k` / `.hud-readout`, the same pairing the web
  // `/me` widgets use for a caption above a reading; the flex sizing
  // and tone colour are tray-specific layout concerns the shared class
  // doesn't know about, so those stay inline.
  return (
    <div className="hud-tile" style={{ flex: '1 1 0', minWidth: 0 }}>
      {/* `.k` is a `.hud-readout .k` descendant rule in hud.css, so the
          label needs its own `.hud-readout` wrapper to pick it up — same
          nesting the KV component below and the web widgets use. */}
      <div className="hud-readout" style={{ marginBottom: 3 }}>
        <span className="k" style={{ marginRight: 0 }}>
          {label}
        </span>
      </div>
      <div
        className="hud-readout"
        style={{
          fontSize: 16,
          fontWeight: 600,
          color: PILL_TONES[tone],
          fontVariantNumeric: 'tabular-nums',
        }}
      >
        {value}
      </div>
    </div>
  );
}

interface StatusDotProps {
  tone?: Tone;
}

const DOT_TONES: Record<Tone, string> = {
  ok: 'var(--ok)',
  warn: 'var(--warn)',
  danger: 'var(--danger)',
  accent: 'var(--accent)',
  info: 'var(--info)',
  dim: 'var(--fg-dim)',
  default: 'var(--fg-muted)',
};

export function StatusDot({ tone = 'ok' }: StatusDotProps) {
  const colour = DOT_TONES[tone];
  return (
    <span
      style={{
        display: 'inline-block',
        width: 8,
        height: 8,
        borderRadius: '50%',
        background: colour,
        boxShadow: `0 0 0 3px ${colour}22`,
        flexShrink: 0,
      }}
    />
  );
}

interface BannerProps {
  tone?: 'warn' | 'info' | 'danger';
  children: ReactNode;
  action?: string;
  onAction?: () => void;
}

// Mirrors the web `.ss-alert--*` treatment: translucent border + tinted
// background derived from the tone token via `color-mix`, instead of a
// full-strength border + hardcoded rgba() background. The hardcoded rgba
// values baked in the dark-theme hue and didn't retint on the Nyx light
// theme (same class of bug as `.ss-btn--danger`, see starstats-tokens.css).
const BANNER_TONES: Record<'warn' | 'info' | 'danger', { border: string; bg: string; fg: string }> = {
  warn: {
    border: 'color-mix(in oklab, var(--warn) 40%, transparent)',
    bg: 'color-mix(in oklab, var(--warn) 8%, var(--bg-elev))',
    fg: 'var(--warn)',
  },
  info: {
    border: 'color-mix(in oklab, var(--info) 40%, transparent)',
    bg: 'color-mix(in oklab, var(--info) 8%, var(--bg-elev))',
    fg: 'var(--info)',
  },
  danger: {
    border: 'color-mix(in oklab, var(--danger) 40%, transparent)',
    bg: 'color-mix(in oklab, var(--danger) 8%, var(--bg-elev))',
    fg: 'var(--danger)',
  },
};

export function Banner({ tone = 'info', children, action, onAction }: BannerProps) {
  const t = BANNER_TONES[tone];
  return (
    <div
      style={{
        display: 'flex',
        alignItems: 'center',
        justifyContent: 'space-between',
        gap: 12,
        padding: '10px 14px',
        borderRadius: 'var(--r-sm)',
        border: `1px solid ${t.border}`,
        background: t.bg,
        color: t.fg,
        fontSize: 13,
      }}
      role="status"
    >
      <span>{children}</span>
      {action && (
        <button
          type="button"
          onClick={onAction}
          style={{
            background: 'transparent',
            color: 'inherit',
            border: '1px solid currentColor',
            borderRadius: 'var(--r-sm)',
            padding: '4px 10px',
            fontWeight: 600,
            fontSize: 12,
            cursor: 'pointer',
            whiteSpace: 'nowrap',
            fontFamily: 'inherit',
          }}
        >
          {action}
        </button>
      )}
    </div>
  );
}

interface FieldProps {
  label: ReactNode;
  hint?: ReactNode;
  children: ReactNode;
}

export function Field({ label, hint, children }: FieldProps) {
  // No small-caps eyebrow treatment: this label repeats once per form
  // field, which is per-field decoration rather than either sanctioned
  // `.ss-eyebrow` use (section category label above an h2, or a stat-tile
  // caption above a single numeric readout) — L8 ruling. Plain label (M4).
  return (
    <label style={{ display: 'flex', flexDirection: 'column', gap: 4 }}>
      <span
        style={{
          fontSize: 10,
          fontWeight: 600,
          color: 'var(--fg-muted)',
        }}
      >
        {label}
      </span>
      {children}
      {hint && (
        <small style={{ fontSize: 11, color: 'var(--fg-dim)', lineHeight: 1.4 }}>{hint}</small>
      )}
    </label>
  );
}

const INPUT_BASE: CSSProperties = {
  background: 'var(--bg)',
  color: 'var(--fg)',
  border: '1px solid var(--border)',
  borderRadius: 'var(--r-sm)',
  padding: '7px 9px',
  fontFamily: 'var(--font-mono)',
  fontSize: 12,
};

export function TextInput(props: InputHTMLAttributes<HTMLInputElement>) {
  const { style, className, ...rest } = props;
  return (
    <input
      {...rest}
      // `.tray-input-focus` (styles.css) supplies the `.ss-input`-equivalent
      // focus treatment — accent border + soft glow ring on `:focus` — since
      // that can't be expressed as an inline style. The keyboard-only accent
      // outline ring is already covered app-wide by the `input:focus-visible`
      // rule (H7) in the same file.
      className={['tray-input-focus', className].filter(Boolean).join(' ')}
      style={{
        ...INPUT_BASE,
        // Mirror the buttons' disabled treatment so a disabled field
        // (e.g. the org-connector URL/token when the connector is off)
        // visibly dims instead of looking fully active.
        opacity: rest.disabled ? 0.55 : 1,
        cursor: rest.disabled ? 'not-allowed' : 'text',
        ...(style ?? {}),
      }}
    />
  );
}

export function PrimaryButton({
  children,
  style,
  ...props
}: ButtonHTMLAttributes<HTMLButtonElement>) {
  return (
    <button
      {...props}
      style={{
        background: 'var(--accent)',
        color: 'var(--accent-fg)',
        border: 'none',
        borderRadius: 'var(--r-sm)',
        padding: '7px 14px',
        fontWeight: 600,
        fontSize: 12,
        cursor: props.disabled ? 'not-allowed' : 'pointer',
        opacity: props.disabled ? 0.55 : 1,
        fontFamily: 'inherit',
        letterSpacing: '0.02em',
        ...(style ?? {}),
      }}
    >
      {children}
    </button>
  );
}

export function GhostButton({
  children,
  style,
  className,
  ...props
}: ButtonHTMLAttributes<HTMLButtonElement>) {
  return (
    <button
      {...props}
      // `.tray-btn-ghost` (styles.css) supplies the base fg-muted/
      // border-strong colouring AND the hover treatment (accent border +
      // accent text, mirroring `.ss-btn--ghost:hover`). Colour/border-color
      // move out of the inline style because inline styles always beat a
      // stylesheet `:hover` rule regardless of specificity, so hover
      // couldn't repaint them any other way without JS state.
      className={['tray-btn-ghost', className].filter(Boolean).join(' ')}
      style={{
        background: 'transparent',
        borderWidth: 1,
        borderStyle: 'solid',
        borderRadius: 'var(--r-sm)',
        padding: '6px 12px',
        fontWeight: 500,
        fontSize: 12,
        cursor: props.disabled ? 'not-allowed' : 'pointer',
        opacity: props.disabled ? 0.55 : 1,
        fontFamily: 'inherit',
        ...(style ?? {}),
      }}
    >
      {children}
    </button>
  );
}

export function DangerButton({
  children,
  style,
  className,
  ...props
}: ButtonHTMLAttributes<HTMLButtonElement>) {
  return (
    <button
      {...props}
      // `.tray-btn-danger` (styles.css) mirrors `.ss-btn--danger`: the
      // border/text colour derive from `--danger` via `color-mix` rather
      // than a hardcoded hex/rgba, so this stays correct on the Nyx light
      // theme (whose `--danger` is a different hue than the dark theme's).
      className={['tray-btn-danger', className].filter(Boolean).join(' ')}
      style={{
        background: 'transparent',
        borderWidth: 1,
        borderStyle: 'solid',
        borderRadius: 'var(--r-sm)',
        padding: '6px 12px',
        fontWeight: 500,
        fontSize: 12,
        cursor: props.disabled ? 'not-allowed' : 'pointer',
        opacity: props.disabled ? 0.55 : 1,
        fontFamily: 'inherit',
        ...(style ?? {}),
      }}
    >
      {children}
    </button>
  );
}
