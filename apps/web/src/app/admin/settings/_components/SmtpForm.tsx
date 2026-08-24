'use client';

/**
 * Admin SMTP config form. Client component because the password
 * field, "send test email" button, and pending state are
 * interactive. Server actions are passed in as props rather than
 * imported from `@/lib/api` (which is `server-only` — the bearer
 * token must never reach the browser bundle).
 *
 * Password field semantics (mirror the server contract):
 *   - empty input + password_set === true  → send null (= keep)
 *   - empty input + password_set === false → send "" (= no auth)
 *   - non-empty input                       → send the typed value
 */

import { BeamAlert } from 'holo';
import { useState, useTransition } from 'react';

/** Local mirror of the API types — duplicated here so this client
 *  module doesn't have to import the server-only `@/lib/api`. The
 *  shapes are pinned to the OpenAPI codegen by the action signatures
 *  the parent page passes in. */
export interface SmtpConfigView {
  host: string;
  port: number;
  username: string;
  password_set: boolean;
  secure: boolean;
  from_addr: string;
  from_name: string;
  web_origin: string;
  enabled: boolean;
}

export interface SmtpConfigPayload {
  host: string;
  port: number;
  username: string;
  /** null = keep existing, "" = clear, non-empty = set */
  password: string | null;
  secure: boolean;
  from_addr: string;
  from_name: string;
  web_origin: string;
  enabled: boolean;
}

export type ActionResult =
  | { kind: 'saved'; config: SmtpConfigView }
  | { kind: 'reloaded'; config: SmtpConfigView }
  | { kind: 'sent'; to: string }
  | { kind: 'error'; message: string };

interface Props {
  initial: SmtpConfigView;
  saveAction: (payload: SmtpConfigPayload) => Promise<ActionResult>;
  /**
   * Send a test email. When `toAddress` is provided, the server sends
   * the test there and skips the "admin email must be verified" gate
   * — used to bootstrap from a state where SMTP has never worked.
   */
  testAction: (toAddress?: string) => Promise<ActionResult>;
  reloadAction: () => Promise<ActionResult>;
}

type Banner =
  | { kind: 'idle' }
  | { kind: 'saved' }
  | { kind: 'sent'; to: string }
  | { kind: 'error'; message: string };

export function SmtpForm({
  initial,
  saveAction,
  testAction,
  reloadAction,
}: Props) {
  const [host, setHost] = useState(initial.host);
  const [port, setPort] = useState(String(initial.port));
  const [username, setUsername] = useState(initial.username);
  const [password, setPassword] = useState('');
  const [secure, setSecure] = useState(initial.secure);
  const [fromAddr, setFromAddr] = useState(initial.from_addr);
  const [fromName, setFromName] = useState(initial.from_name);
  const [webOrigin, setWebOrigin] = useState(initial.web_origin);
  const [enabled, setEnabled] = useState(initial.enabled);
  const [passwordSet, setPasswordSet] = useState(initial.password_set);
  // Optional "send test to" override — empty means "send to my own
  // verified email" (the original behaviour).
  const [testTo, setTestTo] = useState('');
  const [banner, setBanner] = useState<Banner>({ kind: 'idle' });
  const [pending, startTransition] = useTransition();

  function buildPayload(): SmtpConfigPayload {
    // Tri-state: empty field + existing password = "keep".
    let pw: string | null;
    if (password.length === 0) {
      pw = passwordSet ? null : '';
    } else {
      pw = password;
    }
    return {
      host: host.trim(),
      port: parseInt(port, 10) || 0,
      username: username.trim(),
      password: pw,
      secure,
      from_addr: fromAddr.trim(),
      from_name: fromName.trim(),
      web_origin: webOrigin.trim(),
      enabled,
    };
  }

  function applyConfig(c: SmtpConfigView) {
    setHost(c.host);
    setPort(String(c.port));
    setUsername(c.username);
    setPassword(''); // never echo back
    setSecure(c.secure);
    setFromAddr(c.from_addr);
    setFromName(c.from_name);
    setWebOrigin(c.web_origin);
    setEnabled(c.enabled);
    setPasswordSet(c.password_set);
  }

  function handleSave(e: React.FormEvent<HTMLFormElement>) {
    e.preventDefault();
    const payload = buildPayload();
    startTransition(async () => {
      const result = await saveAction(payload);
      if (result.kind === 'saved') {
        applyConfig(result.config);
        setBanner({ kind: 'saved' });
      } else if (result.kind === 'error') {
        setBanner({ kind: 'error', message: result.message });
      }
    });
  }

  function handleTest() {
    const override = testTo.trim();
    startTransition(async () => {
      const result = await testAction(override.length > 0 ? override : undefined);
      if (result.kind === 'sent') {
        setBanner({ kind: 'sent', to: result.to });
      } else if (result.kind === 'error') {
        setBanner({ kind: 'error', message: result.message });
      }
    });
  }

  function handleReload() {
    startTransition(async () => {
      const result = await reloadAction();
      if (result.kind === 'reloaded') {
        applyConfig(result.config);
        setBanner({ kind: 'idle' });
      } else if (result.kind === 'error') {
        setBanner({ kind: 'error', message: result.message });
      }
    });
  }

  return (
    <form
      onSubmit={handleSave}
      style={{ display: 'flex', flexDirection: 'column', gap: 14 }}
    >
      <Field label="Host" hint="e.g. smtp.example.com">
        <input
          type="text"
          value={host}
          onChange={(e) => setHost(e.target.value)}
          autoComplete="off"
          spellCheck={false}
        />
      </Field>

      <div style={{ display: 'flex', gap: 14 }}>
        <Field label="Port" hint="587 / 2525 = STARTTLS  ·  465 = implicit TLS">
          <input
            type="number"
            min={1}
            max={65535}
            value={port}
            onChange={(e) => setPort(e.target.value)}
            style={{ width: 100 }}
          />
        </Field>
        <Field
          label="TLS mode"
          hint={
            secure
              ? 'Implicit TLS — handshake on connect (use with port 465)'
              : 'STARTTLS — plain SMTP, then upgrade (use with port 587 / 2525)'
          }
        >
          <label
            style={{
              display: 'inline-flex',
              alignItems: 'center',
              gap: 8,
            }}
          >
            <input
              type="checkbox"
              checked={secure}
              onChange={(e) => setSecure(e.target.checked)}
            />
            <span>Use implicit TLS (uncheck for STARTTLS)</span>
          </label>
        </Field>
      </div>

      <Field label="Username" hint="leave blank for unauthenticated relay">
        <input
          type="text"
          value={username}
          onChange={(e) => setUsername(e.target.value)}
          autoComplete="off"
          spellCheck={false}
        />
      </Field>

      <Field
        label="Password"
        hint={
          passwordSet
            ? 'Currently set — leave blank to keep, type to replace'
            : 'Not set — leave blank for unauthenticated relay'
        }
      >
        <input
          type="password"
          value={password}
          onChange={(e) => setPassword(e.target.value)}
          placeholder={passwordSet ? '••••••••' : ''}
          autoComplete="new-password"
        />
      </Field>

      <Field label="From address">
        <input
          type="email"
          value={fromAddr}
          onChange={(e) => setFromAddr(e.target.value)}
          autoComplete="off"
        />
      </Field>

      <Field label="From name">
        <input
          type="text"
          value={fromName}
          onChange={(e) => setFromName(e.target.value)}
          autoComplete="off"
        />
      </Field>

      <Field
        label="Web origin"
        hint="https://app.example.com — used for links inside emails"
      >
        <input
          type="url"
          value={webOrigin}
          onChange={(e) => setWebOrigin(e.target.value)}
          autoComplete="off"
          spellCheck={false}
        />
      </Field>

      <Field label="Enabled">
        <label
          style={{ display: 'inline-flex', alignItems: 'center', gap: 8 }}
        >
          <input
            type="checkbox"
            checked={enabled}
            onChange={(e) => setEnabled(e.target.checked)}
          />
          <span>
            Use this config (server-managed). When unchecked, the
            mailer falls back to env-based config or a no-op.
          </span>
        </label>
      </Field>

      <div
        style={{
          display: 'flex',
          gap: 10,
          flexWrap: 'wrap',
          alignItems: 'center',
          marginTop: 6,
        }}
      >
        <button
          type="submit"
          className="hp-btn hp-btn--primary"
          disabled={pending}
        >
          {pending ? 'Working…' : 'Save'}
        </button>
        <input
          type="email"
          inputMode="email"
          placeholder="Send test to… (optional)"
          value={testTo}
          onChange={(e) => setTestTo(e.target.value)}
          disabled={!enabled || pending}
          aria-label="Optional recipient for the test email"
          title="Leave blank to send to your own verified email. Provide an external address (e.g. a personal mailbox) to bootstrap before your admin email is verified."
          style={{
            flex: '1 1 220px',
            minWidth: 200,
            padding: '6px 10px',
            borderRadius: 0,
            border: '1px solid var(--border)',
            background: 'var(--surface)',
            color: 'var(--fg)',
            fontSize: 13,
          }}
        />
        <button
          type="button"
          className="hp-btn"
          onClick={handleTest}
          disabled={!enabled || pending}
          title={
            enabled
              ? testTo.trim().length > 0
                ? `Send a diagnostic email to ${testTo.trim()}`
                : 'Send a diagnostic email to your verified address'
              : 'Enable the config and save before sending a test'
          }
        >
          Send test email
        </button>
        <button
          type="button"
          className="hp-btn hp-btn--ghost"
          onClick={handleReload}
          disabled={pending}
        >
          Reload from server
        </button>
      </div>

      {banner.kind === 'saved' && (
        <BeamAlert tone="good" role="status">
          Saved. Mailer reloaded — any new emails use the updated
          settings immediately.
        </BeamAlert>
      )}
      {banner.kind === 'sent' && (
        <BeamAlert tone="good" role="status">
          Test email sent to <strong>{banner.to}</strong>.
        </BeamAlert>
      )}
      {banner.kind === 'error' && (
        <BeamAlert tone="bad" role="alert">
          {banner.message}
        </BeamAlert>
      )}
    </form>
  );
}

function Field({
  label,
  hint,
  children,
}: {
  label: string;
  hint?: string;
  children: React.ReactNode;
}) {
  return (
    <label style={{ display: 'flex', flexDirection: 'column', gap: 4 }}>
      <span className="hp-kvlabel">{label}</span>
      {children}
      {hint && (
        <span className="hp-field__hint">{hint}</span>
      )}
    </label>
  );
}
