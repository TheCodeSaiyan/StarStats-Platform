/**
 * Admin · Settings — consolidated sitewide config.
 *
 * Absorbs the former /admin/smtp, /admin/appearance and
 * /admin/ship-matrix pages as anchored sections; those routes now
 * redirect here. The three client forms moved verbatim into
 * ./_components — only their page wrappers collapsed.
 *
 * Auth: parent /admin/layout.tsx enforces the role gate. The defensive
 * 401 → login / 403 → /me handling from the old pages is preserved, but
 * it now runs per-section rather than per-page: the three fetches are
 * settled independently (multi-endpoint dashboard invariant) so one
 * failing config degrades to an inline notice instead of blanking the
 * whole console. A 401 on any of them still means the session is gone,
 * which is page-fatal, so that one redirects.
 */

import { redirect } from 'next/navigation';
import { revalidatePath } from 'next/cache';
import {
  ApiCallError,
  getAdminAppearance,
  getShipMatrixConfig,
  getSmtpConfig,
  putShipMatrixConfig,
  putSmtpConfig,
  testSmtp,
  type SmtpConfigRequest,
} from '@/lib/api';
import { logger } from '@/lib/logger';
import { getSession } from '@/lib/session';
import { AdminPageHeader } from '../_components/AdminPageHeader';
import { AppearanceConsole } from './_components/AppearanceConsole';
import {
  ShipMatrixForm,
  type ActionResult as ShipMatrixActionResult,
} from './_components/ShipMatrixForm';
import { SmtpForm, type ActionResult as SmtpActionResult } from './_components/SmtpForm';

export const metadata = { title: 'Settings' };

const sectionTitleStyle: React.CSSProperties = {
  margin: 0,
  fontSize: 17,
  fontWeight: 600,
  letterSpacing: '-0.01em',
};

const sectionNoteStyle: React.CSSProperties = {
  margin: '6px 0 0',
  color: 'var(--fg-muted)',
  fontSize: 13,
  lineHeight: 1.55,
};

export default async function AdminSettingsPage() {
  const session = await getSession();
  if (!session) redirect('/auth/login?next=/admin/settings');

  const [smtp, appearance, shipMatrix] = await Promise.allSettled([
    getSmtpConfig(session.token),
    getAdminAppearance(session.token),
    getShipMatrixConfig(session.token),
  ]);

  // Log each rejection individually with call= and status= so the
  // failing endpoint is named in server logs rather than inferred.
  for (const [call, result] of [
    ['smtp', smtp],
    ['appearance', appearance],
    ['ship-matrix', shipMatrix],
  ] as const) {
    if (result.status === 'rejected') {
      const status =
        result.reason instanceof ApiCallError ? result.reason.status : undefined;
      // An expired session is not a per-section problem.
      if (status === 401) redirect('/auth/login?next=/admin/settings');
      logger.error(
        { err: result.reason, call, status },
        'admin settings section fetch failed',
      );
    }
  }

  async function saveSmtpAction(
    payload: SmtpConfigRequest,
  ): Promise<SmtpActionResult> {
    'use server';
    const s = await getSession();
    if (!s) return { kind: 'error', message: 'no session' };
    try {
      const updated = await putSmtpConfig(payload, s.token);
      revalidatePath('/admin/settings');
      return { kind: 'saved', config: updated };
    } catch (e) {
      if (e instanceof ApiCallError) {
        return {
          kind: 'error',
          message: `${e.body.error}${e.body.detail ? ` — ${e.body.detail}` : ''}`,
        };
      }
      return { kind: 'error', message: String(e) };
    }
  }

  async function testSmtpAction(
    toAddress?: string,
  ): Promise<SmtpActionResult> {
    'use server';
    const s = await getSession();
    if (!s) return { kind: 'error', message: 'no session' };
    try {
      const r = await testSmtp(s.token, toAddress);
      return { kind: 'sent', to: r.sent_to };
    } catch (e) {
      if (e instanceof ApiCallError) {
        return {
          kind: 'error',
          message: `${e.body.error}${e.body.detail ? ` — ${e.body.detail}` : ''}`,
        };
      }
      return { kind: 'error', message: String(e) };
    }
  }

  async function reloadSmtpAction(): Promise<SmtpActionResult> {
    'use server';
    const s = await getSession();
    if (!s) return { kind: 'error', message: 'no session' };
    try {
      const fresh = await getSmtpConfig(s.token);
      return { kind: 'reloaded', config: fresh };
    } catch (e) {
      if (e instanceof ApiCallError) {
        return {
          kind: 'error',
          message: `${e.body.error}${e.body.detail ? ` — ${e.body.detail}` : ''}`,
        };
      }
      return { kind: 'error', message: String(e) };
    }
  }

  async function saveShipMatrixAction(
    mediaEnabled: boolean,
  ): Promise<ShipMatrixActionResult> {
    'use server';
    const s = await getSession();
    if (!s) return { kind: 'error', message: 'no session' };
    try {
      const updated = await putShipMatrixConfig(
        { media_enabled: mediaEnabled },
        s.token,
      );
      revalidatePath('/admin/settings');
      return { kind: 'saved', config: updated };
    } catch (e) {
      if (e instanceof ApiCallError) {
        return {
          kind: 'error',
          message: `${e.body.error}${e.body.detail ? ` — ${e.body.detail}` : ''}`,
        };
      }
      return { kind: 'error', message: String(e) };
    }
  }

  return (
    <div style={{ display: 'flex', flexDirection: 'column', gap: 28 }}>
      <AdminPageHeader
        eyebrow="Admin · settings"
        title="Settings"
        lede="Sitewide configuration: mail transport, appearance defaults, and Ship Matrix enrichment. Each section saves independently."
      />

      <section
        id="smtp"
        style={{ display: 'flex', flexDirection: 'column', gap: 12 }}
      >
        <header>
          <h2 style={sectionTitleStyle}>SMTP configuration</h2>
          <p style={sectionNoteStyle}>
            The mailer hot-reloads as soon as you save — no API restart
            needed. The password is encrypted at rest using the server&apos;s
            KEK and never returned to the browser; leave the field blank to
            keep the existing password. When disabled, the server falls back
            to environment-based config (if any) or a no-op mailer that logs
            sends.
          </p>
        </header>
        {smtp.status === 'fulfilled' ? (
          <SmtpForm
            initial={smtp.value}
            saveAction={saveSmtpAction}
            testAction={testSmtpAction}
            reloadAction={reloadSmtpAction}
          />
        ) : (
          <SectionUnavailable name="SMTP configuration" />
        )}
      </section>

      <section
        id="appearance"
        style={{ display: 'flex', flexDirection: 'column', gap: 12 }}
      >
        <header>
          <h2 style={sectionTitleStyle}>Appearance defaults</h2>
          <p style={sectionNoteStyle}>
            Sitewide defaults for appearance knobs that apply until a
            signed-in user sets a personal override in their own Settings.
          </p>
        </header>
        {appearance.status === 'fulfilled' ? (
          <AppearanceConsole config={appearance.value} />
        ) : (
          <SectionUnavailable name="Appearance defaults" />
        )}
      </section>

      <section
        id="ship-matrix"
        style={{ display: 'flex', flexDirection: 'column', gap: 12 }}
      >
        <header>
          <h2 style={sectionTitleStyle}>Ship Matrix enrichment</h2>
          <p style={sectionNoteStyle}>
            Vehicle specs and descriptions from RSI&apos;s official Ship
            Matrix always populate. This toggle controls whether the official
            ship <strong>images</strong> are surfaced — a comply-on-request
            kill-switch. It takes effect immediately (no redeploy): when off,
            every image request 404s and the gallery is hidden. RSI ship media
            is Cloud Imperium IP shown here under fan-content terms with
            attribution.
          </p>
        </header>
        {shipMatrix.status === 'fulfilled' ? (
          <ShipMatrixForm
            initial={shipMatrix.value}
            saveAction={saveShipMatrixAction}
          />
        ) : (
          <SectionUnavailable name="Ship Matrix enrichment" />
        )}
      </section>
    </div>
  );
}

/**
 * Shown in place of a section whose config fetch failed. Deliberately
 * says the section could not load rather than rendering a form seeded
 * with defaults — a form pre-filled with fabricated values would invite
 * an admin to "save" settings they never actually saw.
 */
function SectionUnavailable({ name }: { name: string }) {
  return (
    <p
      role="status"
      className="ss-card"
      style={{
        margin: 0,
        padding: '20px 24px',
        color: 'var(--fg-muted)',
        fontSize: 13,
      }}
    >
      {name} couldn&apos;t be loaded. The other sections on this page are
      unaffected — reload to try again.
    </p>
  );
}
