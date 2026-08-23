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
import { BeamButton, BeamChip, BeamInput, Plane } from 'holo';

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
  // Render. The four-step flow lives inside one Security pane so the user never
  // leaves /settings. The state machine above is UNCHANGED by the projection
  // port — every cookie, action and branch is the code that shipped, because a
  // 2FA flow is the last place to want incidental breakage. Only the drawing
  // below moved into the beam, and the outer <section> is gone: the settings
  // surface supplies the Pane and the load-bearing `#security` anchor.
  // -------------------------------------------------------------------------

  return (
    <>
      {/* Step 3 — Recovery codes shown right after enable or regenerate. */}
      {recoveryCodes ? (
        <>
          <div className="hp-statusline">
            <BeamChip tone="good" dot>
              On
            </BeamChip>
            <span>
              Save these recovery codes somewhere safe — we can&apos;t show them
              again. Each one works once if you lose your authenticator app.
            </span>
          </div>
          <Plane
            tilt="flat"
            cap="Recovery codes"
            hint="shown once"
            style={{ marginTop: 16 }}
          >
            <div className="hp-codes">
              {recoveryCodes.map((code, i) => (
                <span key={code}>
                  <i>{String(i + 1).padStart(2, '0')}</i>
                  {code}
                </span>
              ))}
            </div>
          </Plane>
          <form action={acknowledgeRecoveryAction} className="hp-formcol">
            <BeamButton type="submit" variant="primary">
              I&apos;ve saved them
            </BeamButton>
          </form>
        </>
      ) : me.totp_enabled ? (
        // Step 4 — Manage view: 2FA already on.
        <>
          <div className="hp-statusline">
            <BeamChip tone="good" dot>
              On
            </BeamChip>
            <span>
              Every sign-in asks for an authentication code from your
              authenticator app, or a one-shot recovery code if you&apos;ve lost
              the app.
            </span>
          </div>

          <Plane
            tilt="flat"
            cap="Regenerate recovery codes"
            style={{ marginTop: 20 }}
          >
            <p className="hp-prose">
              Burn the old set and mint 10 fresh codes. Useful if you think the
              old set leaked, or you&apos;ve used most of them. Re-enter your
              password to confirm.
            </p>
            <form action={regenerateAction} className="hp-formcol">
              <BeamInput
                id="totp-regen-password"
                label="Current password"
                type="password"
                name="password"
                required
                autoComplete="current-password"
              />
              <BeamButton
                type="submit"
                variant="ghost"
                style={{ alignSelf: 'flex-start' }}
              >
                Generate new codes
              </BeamButton>
            </form>
          </Plane>

          <Plane
            tilt="flat"
            cap="Disable two-factor"
            hint="no undo"
            style={{ marginTop: 20 }}
          >
            <p className="hp-prose">
              Removes your authenticator secret and burns all recovery codes.
              Your account drops back to password-only sign-in. Re-enter your
              password to confirm.
            </p>
            <form action={disableAction} className="hp-formcol">
              <BeamInput
                id="totp-disable-password"
                label="Current password"
                type="password"
                name="password"
                required
                autoComplete="current-password"
              />
              <BeamButton
                type="submit"
                variant="danger"
                style={{ alignSelf: 'flex-start' }}
              >
                Turn off 2FA
              </BeamButton>
            </form>
          </Plane>
        </>
      ) : setup ? (
        // Step 2 — Setup in flight: QR + manual secret + verify code.
        <>
          <div className="hp-statusline">
            <BeamChip tone="warn" dot>
              Pairing
            </BeamChip>
            <span>
              Scan into your authenticator app, or enter the secret manually.
              The label <span className="val">{setup.account_label}</span> is
              what appears in the app.
            </span>
          </div>

          <div className="hp-2fa">
            {/* QR generated by OUR server and delivered as a data: URI. It must
                never be produced by handing `provisioning_uri` to a remote
                image service: that URI contains the TOTP shared secret, and
                disclosing it lets the holder mint valid codes forever. This
                previously called api.qrserver.com. */}
            <div className="hp-qr">
              {setupQr ? (
                /* eslint-disable-next-line @next/next/no-img-element */
                <img
                  src={setupQr}
                  alt="Authenticator QR code"
                  width={176}
                  height={176}
                />
              ) : (
                <p
                  className="hp-prose"
                  style={{ margin: 0, textAlign: 'center' }}
                >
                  QR unavailable — enter the setup key below manually.
                </p>
              )}
            </div>

            <div>
              <div className="hp-fieldlabel">Manual secret</div>
              <div className="hp-secret">{setup.secret_base32}</div>
              <p className="hp-prose" style={{ marginTop: 8 }}>
                SHA-1 · 6 digits · 30s period
              </p>

              <div className="hp-fieldlabel" style={{ marginTop: 18 }}>
                Provisioning URI
              </div>
              <div className="hp-secret hp-secret--uri">
                {setup.provisioning_uri}
              </div>
            </div>
          </div>

          <Plane
            tilt="flat"
            cap="Enter the authentication code"
            style={{ marginTop: 20 }}
          >
            <p className="hp-prose">
              Type the 6-digit code your app currently displays. It refreshes
              every 30 seconds.
            </p>
            <form action={confirmAction} className="hp-formcol">
              <BeamInput
                id="totp-code"
                label="Authentication code"
                className="hp-otp"
                type="text"
                name="code"
                required
                inputMode="numeric"
                autoComplete="one-time-code"
                pattern="[0-9]{6}"
                maxLength={6}
                placeholder="123456"
                spellCheck={false}
              />
              <div style={{ display: 'flex', gap: 10, flexWrap: 'wrap' }}>
                <BeamButton type="submit" variant="primary">
                  Verify and enable
                </BeamButton>
              </div>
            </form>
          </Plane>

          <form action={cancelSetupAction} className="hp-formcol">
            <BeamButton
              type="submit"
              variant="ghost"
              style={{ alignSelf: 'flex-start' }}
            >
              Cancel setup
            </BeamButton>
          </form>
        </>
      ) : (
        // Step 1 — Explainer / 2FA off, no setup in flight.
        <>
          <div className="hp-statusline">
            <BeamChip tone="warn" dot>
              Off
            </BeamChip>
            <span>
              Your account is protected only by your password. Adding a second
              factor — a 6-digit code from an authenticator app — stops anyone
              with a stolen password from signing in.
            </span>
          </div>
          <form action={setupAction} className="hp-formcol">
            <BeamButton
              type="submit"
              variant="primary"
              style={{ alignSelf: 'flex-start' }}
            >
              Enable 2FA
            </BeamButton>
          </form>
        </>
      )}
    </>
  );
}
