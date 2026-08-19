import { cookies } from 'next/headers';
import { redirect } from 'next/navigation';
import {
  ApiCallError,
  totpConfirm,
  totpDisable,
  totpRegenerateRecovery,
  totpSetup,
  totpQr,
  type MeResponse,
  type TotpSetupResponse,
} from '@/lib/api';
import { logger } from '@/lib/logger';
import { getSession } from '@/lib/session';

// ---------------------------------------------------------------------------
// Cookie scaffolding — lifted verbatim from the legacy /settings/2fa page.
//
// The 2FA flow needs two short-lived cookies to bridge renders. They are
// scoped to /settings (one level up from the old /settings/2fa path) now
// that the wizard lives inline in the Security section. httpOnly +
// sameSite=lax keeps them off client JS and other origins; everything
// past that follows the same lifecycle the standalone wizard used.
// ---------------------------------------------------------------------------

const SETUP_COOKIE = 'totp-setup';
const RECOVERY_COOKIE = 'totp-recovery';
const SETUP_COOKIE_TTL_SECS = 10 * 60;
const RECOVERY_COOKIE_TTL_SECS = 2 * 60;
const COOKIE_PATH = '/settings';

interface SetupCookiePayload {
  secret_base32: string;
  provisioning_uri: string;
  account_label: string;
}

// ---------------------------------------------------------------------------
// Style helpers — kept local so the section can be dropped into a card
// without inheriting page-level state. Matches the surrounding card grammar
// used by settings/page.tsx (cardHeaderStyle/cardBodyStyle equivalents).
// ---------------------------------------------------------------------------

const headerStyle: React.CSSProperties = { padding: '20px 24px 0' };

const bodyStyle: React.CSSProperties = {
  padding: '16px 24px 22px',
  display: 'flex',
  flexDirection: 'column',
  gap: 14,
};

const eyebrowStyle: React.CSSProperties = { marginBottom: 6 };

const titleStyle: React.CSSProperties = {
  margin: 0,
  fontSize: 17,
  fontWeight: 600,
  letterSpacing: '-0.01em',
  color: 'var(--fg)',
};

const subTitleStyle: React.CSSProperties = {
  margin: '0 0 6px',
  fontSize: 15,
  fontWeight: 600,
  letterSpacing: '-0.01em',
  color: 'var(--fg)',
};

const mutedStyle: React.CSSProperties = {
  margin: 0,
  color: 'var(--fg-muted)',
  fontSize: 13,
  lineHeight: 1.55,
};

const dimStyle: React.CSSProperties = {
  color: 'var(--fg-dim)',
  fontSize: 12,
};

const formStyle: React.CSSProperties = {
  display: 'flex',
  flexDirection: 'column',
  gap: 12,
  margin: 0,
};

// ---------------------------------------------------------------------------
// Component
// ---------------------------------------------------------------------------

/**
 * Inline 2FA wizard. Drop-in replacement for the previous standalone
 * /settings/2fa route. Renders one of four states based on
 * `me.totp_enabled` plus the `totp-setup` / `totp-recovery` cookies:
 *
 *   1. Off          (explainer / begin enrolment CTA)
 *   2. Setup        (QR + manual secret + verify code)
 *   3. Recovery     (10 plaintext codes shown exactly once)
 *   4. Manage       (regenerate / disable)
 *
 * All server actions redirect back to `/settings#security` so the user
 * stays inside the Settings page across the entire flow.
 */
export async function SecuritySection({ me }: { me: MeResponse }) {
  const jar = await cookies();
  const setupRaw = jar.get(SETUP_COOKIE)?.value;
  const recoveryRaw = jar.get(RECOVERY_COOKIE)?.value;

  // Parse cookies defensively — a tampered or stale cookie should fall
  // back to the "no cookie" branch rather than throw.
  let setup: SetupCookiePayload | null = null;
  if (setupRaw) {
    try {
      setup = JSON.parse(setupRaw) as SetupCookiePayload;
    } catch {
      setup = null;
    }
  }
  // QR for the pending enrolment, rendered by OUR server. Deliberately
  // fetched rather than stored beside the rest of the payload: the
  // enrolment cookie is capped near 4 KB and a QR data URI is ~9 KB, so
  // storing it there made the browser drop the cookie and lose the
  // enrolment silently.
  let setupQr: string | null = null;
  if (setup?.provisioning_uri) {
    const qrSession = await getSession();
    if (qrSession) setupQr = await totpQr(setup.provisioning_uri, qrSession.token);
  }

  let recoveryCodes: string[] | null = null;
  if (recoveryRaw) {
    try {
      const parsed = JSON.parse(recoveryRaw) as unknown;
      if (
        Array.isArray(parsed) &&
        parsed.every((c) => typeof c === 'string')
      ) {
        recoveryCodes = parsed;
      }
    } catch {
      recoveryCodes = null;
    }
  }

  // -------------------------------------------------------------------------
  // Server actions. Each redirects back to /settings#security so the
  // Security card stays in view after the page revalidates.
  // -------------------------------------------------------------------------

  async function setupAction() {
    'use server';
    const session = await getSession();
    if (!session) redirect('/auth/login?next=/settings');
    let resp: TotpSetupResponse;
    try {
      resp = await totpSetup(session.token);
    } catch (e) {
      if (e instanceof ApiCallError && e.status === 401) {
        redirect('/auth/login?next=/settings');
      }
      if (e instanceof ApiCallError && e.status === 409) {
        // Already enabled — fall through to the manage view.
        redirect('/settings#security');
      }
      logger.error({ err: e }, 'totp setup failed');
      redirect('/settings?error=unexpected#security');
    }
    const payload: SetupCookiePayload = {
      secret_base32: resp.secret_base32,
      provisioning_uri: resp.provisioning_uri,
      account_label: resp.account_label,
    };
    const jar = await cookies();
    jar.set({
      name: SETUP_COOKIE,
      value: JSON.stringify(payload),
      httpOnly: true,
      secure: process.env.NODE_ENV === 'production',
      sameSite: 'lax',
      path: COOKIE_PATH,
      maxAge: SETUP_COOKIE_TTL_SECS,
    });
    redirect('/settings#security');
  }

  async function cancelSetupAction() {
    'use server';
    const jar = await cookies();
    jar.delete({ name: SETUP_COOKIE, path: COOKIE_PATH });
    redirect('/settings#security');
  }

  async function confirmAction(formData: FormData) {
    'use server';
    const session = await getSession();
    if (!session) redirect('/auth/login?next=/settings');
    const code = String(formData.get('code') ?? '').trim();
    if (code === '') {
      redirect('/settings?error=invalid_code#security');
    }
    let codes: string[];
    try {
      const resp = await totpConfirm(session.token, { code });
      codes = resp.recovery_codes;
    } catch (e) {
      if (e instanceof ApiCallError) {
        if (e.status === 401) redirect('/auth/login?next=/settings');
        if (e.status === 400) redirect('/settings?error=no_setup#security');
        if (e.status === 409) {
          redirect('/settings?error=already_enabled#security');
        }
        if (e.status === 422) {
          redirect('/settings?error=invalid_code#security');
        }
      }
      logger.error({ err: e }, 'totp confirm failed');
      redirect('/settings?error=unexpected#security');
    }
    const jar = await cookies();
    jar.delete({ name: SETUP_COOKIE, path: COOKIE_PATH });
    jar.set({
      name: RECOVERY_COOKIE,
      value: JSON.stringify(codes),
      httpOnly: true,
      secure: process.env.NODE_ENV === 'production',
      sameSite: 'lax',
      path: COOKIE_PATH,
      maxAge: RECOVERY_COOKIE_TTL_SECS,
    });
    redirect('/settings?status=totp_enabled#security');
  }

  async function acknowledgeRecoveryAction() {
    'use server';
    const jar = await cookies();
    jar.delete({ name: RECOVERY_COOKIE, path: COOKIE_PATH });
    redirect('/settings?status=totp_ack#security');
  }

  async function disableAction(formData: FormData) {
    'use server';
    const session = await getSession();
    if (!session) redirect('/auth/login?next=/settings');
    const password = String(formData.get('password') ?? '');
    try {
      await totpDisable(session.token, { password });
    } catch (e) {
      if (e instanceof ApiCallError) {
        if (e.status === 401) {
          // 401 here means the *password* check failed; the bearer was
          // already validated to even reach the handler.
          redirect('/settings?error=invalid_credentials#security');
        }
        if (e.status === 409) {
          redirect('/settings?error=not_enabled#security');
        }
      }
      logger.error({ err: e }, 'totp disable failed');
      redirect('/settings?error=unexpected#security');
    }
    const jar = await cookies();
    jar.delete({ name: SETUP_COOKIE, path: COOKIE_PATH });
    jar.delete({ name: RECOVERY_COOKIE, path: COOKIE_PATH });
    redirect('/settings?status=totp_disabled#security');
  }

  async function regenerateAction(formData: FormData) {
    'use server';
    const session = await getSession();
    if (!session) redirect('/auth/login?next=/settings');
    const password = String(formData.get('password') ?? '');
    let codes: string[];
    try {
      const resp = await totpRegenerateRecovery(session.token, { password });
      codes = resp.recovery_codes;
    } catch (e) {
      if (e instanceof ApiCallError) {
        if (e.status === 401) {
          redirect('/settings?error=invalid_credentials#security');
        }
        if (e.status === 409) {
          redirect('/settings?error=not_enabled#security');
        }
      }
      logger.error({ err: e }, 'totp regenerate recovery failed');
      redirect('/settings?error=unexpected#security');
    }
    const jar = await cookies();
    jar.set({
      name: RECOVERY_COOKIE,
      value: JSON.stringify(codes),
      httpOnly: true,
      secure: process.env.NODE_ENV === 'production',
      sameSite: 'lax',
      path: COOKIE_PATH,
      maxAge: RECOVERY_COOKIE_TTL_SECS,
    });
    redirect('/settings?status=totp_regenerated#security');
  }

  // -------------------------------------------------------------------------
  // Render. The four-step flow lives inside a single Security card so the
  // user never leaves /settings. We keep the legacy state machine — it is
  // already well-tested and a UI rewrite isn't in scope.
  // -------------------------------------------------------------------------

  return (
    <section className="ss-card" id="security">
      <header style={headerStyle}>
        <div className="ss-eyebrow" style={eyebrowStyle}>
          Security
        </div>
        <h2 style={titleStyle}>Two-factor authentication</h2>
      </header>
      <div style={bodyStyle}>
        {/* Step 3 — Recovery codes shown right after enable or regenerate. */}
        {recoveryCodes ? (
          <>
            <div
              style={{
                display: 'flex',
                gap: 10,
                alignItems: 'flex-start',
              }}
            >
              <span className="ss-badge ss-badge--ok">
                <span className="ss-badge-dot" />
                On
              </span>
              <span
                style={{
                  color: 'var(--fg-muted)',
                  fontSize: 13,
                  lineHeight: 1.5,
                }}
              >
                Save these recovery codes somewhere safe — we can&apos;t
                show them again. Each one works once if you lose your
                authenticator app.
              </span>
            </div>
            <div
              className="ss-secret"
              style={{
                padding: 16,
                alignItems: 'flex-start',
                flexDirection: 'column',
                gap: 6,
              }}
            >
              <div
                className="mono"
                style={{
                  display: 'grid',
                  gridTemplateColumns: 'repeat(2, minmax(0, 1fr))',
                  gap: '6px 24px',
                  width: '100%',
                  fontSize: 13,
                  lineHeight: 1.7,
                  letterSpacing: '0.04em',
                  color: 'var(--fg)',
                }}
              >
                {recoveryCodes.map((code, i) => (
                  <span key={code}>
                    <span style={{ color: 'var(--fg-dim)', marginRight: 8 }}>
                      {String(i + 1).padStart(2, '0')}
                    </span>
                    {code}
                  </span>
                ))}
              </div>
            </div>
            <form action={acknowledgeRecoveryAction} style={formStyle}>
              <button
                type="submit"
                className="ss-btn ss-btn--primary"
                style={{ alignSelf: 'flex-start' }}
              >
                I&apos;ve saved them
              </button>
            </form>
          </>
        ) : me.totp_enabled ? (
          // Step 4 — Manage view: 2FA already on.
          <>
            <div
              style={{
                display: 'flex',
                gap: 10,
                alignItems: 'flex-start',
              }}
            >
              <span className="ss-badge ss-badge--ok">
                <span className="ss-badge-dot" />
                On
              </span>
              <span
                style={{
                  color: 'var(--fg-muted)',
                  fontSize: 13,
                  lineHeight: 1.5,
                }}
              >
                Every sign-in asks for an authentication code from your
                authenticator app, or a one-shot recovery code if
                you&apos;ve lost the app.
              </span>
            </div>

            <hr className="ss-rule" />

            <div>
              <h3 style={subTitleStyle}>Regenerate recovery codes</h3>
              <p style={mutedStyle}>
                Burn the old set and mint 10 fresh codes. Useful if you
                think the old set leaked, or you&apos;ve used most of
                them. Re-enter your password to confirm.
              </p>
            </div>
            <form action={regenerateAction} style={formStyle}>
              <label className="ss-label">
                <span className="ss-label-text">Current password</span>
                <input
                  className="ss-input"
                  type="password"
                  name="password"
                  required
                  autoComplete="current-password"
                />
              </label>
              <button
                type="submit"
                className="ss-btn ss-btn--ghost"
                style={{ alignSelf: 'flex-start' }}
              >
                Generate new codes
              </button>
            </form>

            <hr className="ss-rule" />

            <div>
              <div
                className="ss-eyebrow"
                style={{ marginBottom: 6, color: 'var(--danger)' }}
              >
                Danger zone
              </div>
              <h3 style={subTitleStyle}>Disable two-factor</h3>
              <p style={mutedStyle}>
                Removes your authenticator secret and burns all recovery
                codes. Your account drops back to password-only sign-in.
                Re-enter your password to confirm.
              </p>
            </div>
            <form action={disableAction} style={formStyle}>
              <label className="ss-label">
                <span className="ss-label-text">Current password</span>
                <input
                  className="ss-input"
                  type="password"
                  name="password"
                  required
                  autoComplete="current-password"
                />
              </label>
              <button
                type="submit"
                className="ss-btn ss-btn--danger"
                style={{ alignSelf: 'flex-start' }}
              >
                Turn off 2FA
              </button>
            </form>
          </>
        ) : setup ? (
          // Step 2 — Setup in flight: QR + manual secret + verify code.
          <>
            <div
              style={{
                display: 'flex',
                gap: 10,
                alignItems: 'flex-start',
              }}
            >
              <span className="ss-badge ss-badge--warn">
                <span className="ss-badge-dot" />
                Pairing
              </span>
              <span
                style={{
                  color: 'var(--fg-muted)',
                  fontSize: 13,
                  lineHeight: 1.5,
                }}
              >
                Scan into your authenticator app, or enter the secret
                manually. The label{' '}
                <span className="mono">{setup.account_label}</span> is
                what appears in the app.
              </span>
            </div>

            <div
              style={{
                display: 'grid',
                gridTemplateColumns: 'minmax(0, 200px) 1fr',
                gap: 18,
                alignItems: 'start',
                marginTop: 4,
              }}
            >
              {/* QR generated by OUR server and delivered as a data: URI.
                  It must never be produced by handing `provisioning_uri`
                  to a remote image service: that URI contains the TOTP
                  shared secret, and disclosing it lets the holder mint
                  valid codes forever. This previously called
                  api.qrserver.com. */}
              <div
                style={{
                  background: 'var(--bg-elev)',
                  border: '1px solid var(--border)',
                  borderRadius: 'var(--r-sm)',
                  padding: 12,
                  display: 'grid',
                  placeItems: 'center',
                }}
              >
                {setupQr ? (
                  /* eslint-disable-next-line @next/next/no-img-element */
                  <img
                    src={setupQr}
                    alt="Authenticator QR code"
                    width={176}
                    height={176}
                    style={{
                      display: 'block',
                      /* White, not a theme token: a themed backdrop can
                         invert or wash out the code and scanners reject
                         it. The SVG carries its own quiet zone. */
                      background: '#ffffff',
                      borderRadius: 4,
                    }}
                  />
                ) : (
                  <p
                    style={{
                      fontSize: 12,
                      color: 'var(--fg-muted)',
                      maxWidth: 176,
                      textAlign: 'center',
                      margin: 0,
                    }}
                  >
                    QR unavailable — enter the setup key below manually.
                  </p>
                )}
              </div>

              <div
                style={{
                  display: 'flex',
                  flexDirection: 'column',
                  gap: 12,
                }}
              >
                <div>
                  <span className="ss-label-text">Manual secret</span>
                  <div className="ss-secret" style={{ marginTop: 6 }}>
                    <span className="ss-secret-code mono">
                      {setup.secret_base32}
                    </span>
                  </div>
                  <small
                    style={{ ...dimStyle, display: 'block', marginTop: 6 }}
                  >
                    SHA-1 · 6 digits · 30s period
                  </small>
                </div>

                <div>
                  <span className="ss-label-text">Provisioning URI</span>
                  <div className="ss-secret" style={{ marginTop: 6 }}>
                    <span
                      className="ss-secret-code mono"
                      style={{ fontSize: 12, letterSpacing: 0 }}
                    >
                      {setup.provisioning_uri}
                    </span>
                  </div>
                </div>
              </div>
            </div>

            <hr className="ss-rule" />

            <div>
              <h3 style={subTitleStyle}>Enter the authentication code</h3>
              <p style={mutedStyle}>
                Type the 6-digit code your app currently displays. It
                refreshes every 30 seconds.
              </p>
            </div>
            <form action={confirmAction} style={formStyle}>
              <label className="ss-label">
                <span className="ss-label-text">Authentication code</span>
                <input
                  className="ss-input mono"
                  type="text"
                  name="code"
                  required
                  inputMode="numeric"
                  autoComplete="one-time-code"
                  pattern="[0-9]{6}"
                  maxLength={6}
                  placeholder="123456"
                  spellCheck={false}
                  style={{
                    letterSpacing: '0.4em',
                    fontSize: 18,
                    textAlign: 'center',
                    maxWidth: 220,
                  }}
                />
              </label>
              <div
                style={{
                  display: 'flex',
                  gap: 10,
                  flexWrap: 'wrap',
                }}
              >
                <button type="submit" className="ss-btn ss-btn--primary">
                  Verify and enable
                </button>
              </div>
            </form>

            <hr className="ss-rule" />
            <form action={cancelSetupAction} style={formStyle}>
              <button
                type="submit"
                className="ss-btn ss-btn--ghost"
                style={{ alignSelf: 'flex-start' }}
              >
                Cancel setup
              </button>
            </form>
          </>
        ) : (
          // Step 1 — Explainer / 2FA off, no setup in flight.
          <>
            <div
              style={{
                display: 'flex',
                gap: 10,
                alignItems: 'flex-start',
              }}
            >
              <span className="ss-badge ss-badge--warn">
                <span className="ss-badge-dot" />
                Off
              </span>
              <span
                style={{
                  color: 'var(--fg-muted)',
                  fontSize: 13,
                  lineHeight: 1.5,
                }}
              >
                Your account is protected only by your password. Adding a
                second factor — a 6-digit code from an authenticator app
                — stops anyone with a stolen password from signing in.
              </span>
            </div>
            <form action={setupAction} style={formStyle}>
              <button
                type="submit"
                className="ss-btn ss-btn--primary"
                style={{ alignSelf: 'flex-start' }}
              >
                Enable 2FA
              </button>
            </form>
          </>
        )}
      </div>
    </section>
  );
}
