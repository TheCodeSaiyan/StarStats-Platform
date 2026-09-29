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
  getRetentionPolicies,
  getShipMatrixConfig,
  getSmtpConfig,
  putRetentionPolicy,
  putShipMatrixConfig,
  putSmtpConfig,
  testSmtp,
  type SmtpConfigRequest,
} from '@/lib/api';
import { BeamAlert } from 'holo';
import { ConfirmSubmitButton } from '@/components/forms/ConfirmSubmitButton';
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

/** Outcome chips for the retention form, keyed by `?retention=`. */
const RETENTION_NOTICES: Record<string, { tone: 'good' | 'bad'; text: string }> = {
  saved: { tone: 'good', text: 'Retention saved. The next daily purge uses it.' },
  invalid_retention_days: { tone: 'bad', text: 'Pick a whole number of days from 1 to 3650, or unlimited.' },
  unknown_tier: { tone: 'bad', text: 'That tier does not exist.' },
  error: { tone: 'bad', text: 'Retention was not saved. Try again.' },
};

/** A tier's window as people read it. */
function windowLabel(days: number | null | undefined): string {
  return days == null ? 'unlimited' : `${days} day${days === 1 ? '' : 's'}`;
}

export default async function AdminSettingsPage(props: {
  searchParams: Promise<{ retention?: string }>;
}) {
  const session = await getSession();
  if (!session) redirect('/auth/login?next=/admin/settings');
  const { retention: retentionStatus } = await props.searchParams;

  const [smtp, appearance, shipMatrix, retention] = await Promise.allSettled([
    getSmtpConfig(session.token),
    getAdminAppearance(session.token),
    getShipMatrixConfig(session.token),
    getRetentionPolicies(session.token),
  ]);

  // Log each rejection individually with call= and status= so the
  // failing endpoint is named in server logs rather than inferred.
  for (const [call, result] of [
    ['smtp', smtp],
    ['appearance', appearance],
    ['ship-matrix', shipMatrix],
    ['retention', retention],
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

  async function setRetentionAction(formData: FormData) {
    'use server';
    const s = await getSession();
    if (!s) redirect('/auth/login?next=/admin/settings');
    const tier = String(formData.get('tier') ?? '');
    const unlimited = formData.get('unlimited') === 'on';
    const days = Number(formData.get('days'));
    let outcome = 'saved';
    try {
      const res = await putRetentionPolicy(tier, unlimited ? null : days, s.token);
      // The chip follows what the server now holds, not what was asked.
      const now = res.policies.find((p) => p.tier === tier);
      const held = now?.retention_days ?? null;
      if (!now || held !== (unlimited ? null : days)) outcome = 'error';
    } catch (e) {
      const code = e instanceof ApiCallError ? e.body.error : undefined;
      outcome = code && code in RETENTION_NOTICES ? code : 'error';
      logger.error({ err: e, call: 'retention.set', tier }, 'retention policy update failed');
    }
    revalidatePath('/admin/settings');
    redirect(`/admin/settings?retention=${outcome}#retention`);
  }

  const retentionNotice = retentionStatus ? RETENTION_NOTICES[retentionStatus] : undefined;

  return (
    <div style={{ display: 'flex', flexDirection: 'column', gap: 28 }}>
      <AdminPageHeader
        eyebrow="Admin · settings"
        title="Settings"
        lede="Sitewide configuration: event retention, mail transport, appearance defaults, and Ship Matrix enrichment. Each section saves independently."
      />

      <section
        id="retention"
        style={{ display: 'flex', flexDirection: 'column', gap: 12 }}
      >
        <header>
          <h2 className="hp-sectiontitle">Event retention</h2>
          <p className="hp-fine">
            How long players&apos; uploaded game events are kept, per tier. A
            daily purge deletes events uploaded longer ago than the window;
            supporters are the tier while their support is active. Shortening
            a window deletes older events at the next purge, and they cannot
            be recovered. The settings page and privacy notice describe these
            windows, so change them together.
          </p>
        </header>
        {retentionNotice ? (
          <BeamAlert tone={retentionNotice.tone}>{retentionNotice.text}</BeamAlert>
        ) : null}
        {retention.status === 'fulfilled' ? (
          retention.value.policies.map((p) => (
            <form
              key={p.tier}
              action={setRetentionAction}
              className="hp-formrow"
              data-testid={`retention-${p.tier}`}
              style={{ display: 'flex', flexWrap: 'wrap', gap: 12, alignItems: 'flex-end' }}
            >
              <input type="hidden" name="tier" value={p.tier} />
              <p className="hp-prose" style={{ margin: 0, minWidth: 180 }}>
                <strong>{p.tier}</strong>: {windowLabel(p.retention_days)}
              </p>
              <label className="ss-label">
                <span className="ss-label-text">Days</span>
                <input
                  className="ss-input"
                  type="number"
                  name="days"
                  min={1}
                  max={3650}
                  defaultValue={p.retention_days ?? ''}
                />
              </label>
              <label className="hp-check">
                <input type="checkbox" name="unlimited" defaultChecked={p.retention_days == null} />{' '}
                Unlimited
              </label>
              <ConfirmSubmitButton
                className="hp-btn"
                confirm="Shortening a window deletes events outside it at the next purge, for good. Save?"
              >
                Save
              </ConfirmSubmitButton>
            </form>
          ))
        ) : (
          <SectionUnavailable name="Event retention" />
        )}
      </section>

      <section
        id="smtp"
        style={{ display: 'flex', flexDirection: 'column', gap: 12 }}
      >
        <header>
          <h2 className="hp-sectiontitle">SMTP configuration</h2>
          <p className="hp-fine">
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
          <h2 className="hp-sectiontitle">Appearance defaults</h2>
          <p className="hp-fine">
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
          <h2 className="hp-sectiontitle">Ship Matrix enrichment</h2>
          <p className="hp-fine">
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
