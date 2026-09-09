import React from 'react';

/**
 * Last line of defence against a silent white window.
 *
 * The tray had no error boundary at all. A throw anywhere in the render
 * tree therefore unmounted everything and left the webview showing a blank
 * white rectangle — visually identical to the 0.1.14 AppImage bug, which
 * came from an entirely different cause (a bundled `libwayland-client`
 * breaking Mesa's EGL, see `release.yml`). Telling those two apart cost an
 * evening and six wrong hypotheses. Anything that puts a message on screen
 * instead of nothing at all is worth more than it looks.
 *
 * Scope, honestly stated: this catches errors thrown while RENDERING. It
 * cannot catch a module-level `SyntaxError`, a failed bundle load, or a
 * crash in the webview itself — in those cases React never runs and the
 * window is blank regardless. The CSP (`script-src 'self'`) also rules out
 * the usual trick of an inline pre-bundle `window.onerror` shim. So a blank
 * window still means "React never started"; a panel means "React started
 * and something threw", and that distinction is the whole point.
 *
 * Styling is inline against the theme tokens rather than classNames: an
 * unstyled boundary is the one component that must never depend on a
 * stylesheet having loaded, and `.ss-*` classes with no matching CSS pass
 * every test gate silently (see `.claude/rules/web-testing.md`).
 */
interface Props {
  children: React.ReactNode;
}

interface State {
  error: Error | null;
}

export class BootErrorBoundary extends React.Component<Props, State> {
  state: State = { error: null };

  static getDerivedStateFromError(error: Error): State {
    return { error };
  }

  componentDidCatch(error: Error, info: React.ErrorInfo): void {
    // Goes to the webview console and, on Linux, to the terminal the app
    // was started from — which is where anyone debugging a blank window is
    // already looking.
    console.error('tray render error', error, info.componentStack);
  }

  render(): React.ReactNode {
    const { error } = this.state;
    if (!error) return this.props.children;

    return (
      <div
        role="alert"
        style={{
          padding: 24,
          fontSize: 13,
          lineHeight: 1.5,
          color: 'var(--fg)',
          background: 'var(--bg)',
          minHeight: '100vh',
          fontFamily: 'inherit',
        }}
      >
        <div
          style={{
            fontWeight: 600,
            fontSize: 15,
            marginBottom: 8,
            color: 'var(--fg)',
          }}
        >
          StarStats hit an error and stopped drawing.
        </div>
        <p style={{ color: 'var(--fg-muted)', margin: '0 0 14px', maxWidth: '60ch' }}>
          This is a bug in the app, not something you did. Reloading usually
          gets you back in; if it keeps happening, the message below is the
          useful part of a bug report.
        </p>
        <pre
          style={{
            margin: '0 0 16px',
            padding: 12,
            borderRadius: 4,
            border: '1px solid var(--border)',
            background: 'var(--bg-elev, transparent)',
            color: 'var(--fg)',
            fontSize: 12,
            whiteSpace: 'pre-wrap',
            wordBreak: 'break-word',
            maxHeight: 220,
            overflow: 'auto',
          }}
        >
          {error.message || String(error)}
        </pre>
        <button
          type="button"
          onClick={() => window.location.reload()}
          style={{
            padding: '6px 14px',
            fontSize: 13,
            cursor: 'pointer',
            color: 'var(--fg)',
            background: 'transparent',
            border: '1px solid var(--border)',
            borderRadius: 4,
          }}
        >
          Reload
        </button>
      </div>
    );
  }
}

export default BootErrorBoundary;
