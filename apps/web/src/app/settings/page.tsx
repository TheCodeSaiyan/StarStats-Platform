import { revalidatePath } from 'next/cache';
import { redirect } from 'next/navigation';
import {
  ApiCallError,
  changePassword,
  deleteAccount,
  emailChangeStart,
  getMe,
  getMyHangar,
  getMyProfile,
  getPreferences,
  putPreferences,
  refreshProfile,
  refreshRsiOrgs,
  resendVerification,
  rsiVerifyCheck,
  rsiVerifyStart,
  type HangarSnapshot,
  type MeResponse,
  type ProfileResponse,
  type RsiStartResponse,
} from '@/lib/api';
import { logger } from '@/lib/logger';
import { clearSession, getSession } from '@/lib/session';
import { getTheme, isTheme, setTheme, type Theme } from '@/lib/theme';
import {
  DEFAULT_WAVE_SPEED,
  isWaveSpeed,
  type WaveSpeed,
} from '@/lib/wave-speed';
import { SecuritySection } from './_components/SecuritySection';
import {
  BeamButton,
  BeamChip,
  BeamInput,
  HoloKV,
  Plane,
  type Calibration,
} from 'holo';
import { navSections } from '@/lib/nav';
import { setCalibrationAction } from '@/app/me/_projection/actions';
import {
  SettingsProjection,
  type SettingsSection,
} from './_projection/SettingsProjection';
import { TimezoneField, WaveSpeedField } from './_projection/controls';
import { HangarPane } from './_projection/HangarPane';
import { SETTINGS_GROUPS } from './_projection/groups';

export const metadata = { title: "Settings" };

interface SearchParams {
  status?: string;
  error?: string;
}

// ----- Layout style helpers ------------------------------------------------
//
// We're inside `.ss-main` (provided by the app shell in layout.tsx), so the
// page itself just needs a centered stack of cards. No `.dashboard` wrapper.














// ---------------------------------------------------------------------------

export default async function SettingsPage(props: {
  searchParams: Promise<SearchParams>;
}) {
  const session = await getSession();
  if (!session) redirect('/auth/login?next=/settings');

  const { status, error } = await props.searchParams;

  // /v1/auth/me is the source of truth — the cookie may be stale.
  let me: MeResponse;
  try {
    me = await getMe(session.token);
  } catch (e) {
    if (e instanceof ApiCallError && e.status === 401) {
      redirect('/auth/login?next=/settings');
    }
    throw e;
  }

  // Sharing state moved to /sharing in 0.0.4-beta — no longer
  // loaded here. See apps/web/src/app/sharing/page.tsx.

  // Pull (or issue) the RSI verification code only when the handle
  // isn't proven yet — already-verified users don't need a code.
  // Failure here degrades to "couldn't load" rather than throwing,
  // because verification is opt-in and the rest of the page is
  // unrelated.
  let rsiState: RsiStartResponse | null = null;
  let rsiLoadFailed = false;
  if (!me.rsi_verified) {
    try {
      rsiState = await rsiVerifyStart(session.token);
    } catch (e) {
      if (e instanceof ApiCallError && e.status === 401) {
        redirect('/auth/login?next=/settings');
      }
      logger.warn({ err: e }, 'rsi verify start failed');
      rsiLoadFailed = true;
    }
  }

  // Active theme. Source-of-truth order:
  //   1. server-side preferences row (follows user across devices)
  //   2. local `ss-theme` cookie (last-write-wins for this browser)
  //   3. DEFAULT_THEME (Stanton)
  // The PUT /v1/me/preferences endpoint is fresh from Wave 8.3 backend —
  // if it errors, fall through to the cookie so the page still paints.
  let activeTheme: Theme = await getTheme();
  // Wave speed has no cookie fallback (it's not read pre-auth the way
  // theme is) — absent/unreadable prefs just mean "sitewide default",
  // which DEFAULT_WAVE_SPEED approximates for this authed-only control.
  let activeWaveSpeed: WaveSpeed = DEFAULT_WAVE_SPEED;
  let storedTimezone: string | null = null;
  try {
    const prefs = await getPreferences(session.token);
    if (isTheme(prefs.theme)) activeTheme = prefs.theme;
    if (isWaveSpeed(prefs.theme_wave_speed)) {
      activeWaveSpeed = prefs.theme_wave_speed;
    }
    storedTimezone = prefs.timezone ?? null;
  } catch (e) {
    if (e instanceof ApiCallError && e.status === 401) {
      redirect('/auth/login?next=/settings');
    }
    logger.warn({ err: e }, 'load preferences failed');
    // fall through — cookie value (or DEFAULT_THEME) already in activeTheme
  }

  // Profile snapshot — only meaningful for verified users (the
  // refresh endpoint 422s otherwise, and the snapshot cache is keyed
  // off the verified handle). 404 = "no snapshot yet"; any other
  // error degrades to "load failed" so the rest of the page still
  // renders.
  let profile: ProfileResponse | null = null;
  let profileLoadFailed = false;
  if (me.rsi_verified) {
    try {
      profile = await getMyProfile(session.token);
    } catch (e) {
      if (e instanceof ApiCallError && e.status === 401) {
        redirect('/auth/login?next=/settings');
      }
      if (!(e instanceof ApiCallError) || e.status !== 404) {
        logger.warn({ err: e }, 'load my profile snapshot failed');
        profileLoadFailed = true;
      }
    }
  }

  // Hangar snapshot — pushed by the tray client, not the website.
  // Surfaced here so users can confirm the tray is talking to the
  // server without launching the tray itself. `getMyHangar` already
  // converts 404 ("no_hangar_yet") into a typed null, so the only
  // surprise we have to catch is a 401 (session expired).
  // Hangar sync is independent of RSI verification — pairing a
  // device is sufficient.
  let hangar: HangarSnapshot | null = null;
  try {
    hangar = await getMyHangar(session.token);
  } catch (e) {
    if (e instanceof ApiCallError && e.status === 401) {
      redirect('/auth/login?next=/settings');
    }
    logger.warn({ err: e }, 'load hangar snapshot failed');
  }

  async function themeAction(formData: FormData) {
    'use server';
    const session = await getSession();
    if (!session) redirect('/auth/login?next=/settings#theme');
    const raw = formData.get('theme');
    if (!isTheme(raw)) {
      // Form was tampered with or submitted without a button value —
      // ignore silently rather than error out, themes aren't load-bearing.
      redirect('/settings?error=invalid_theme#theme');
    }
    // setTheme writes the cookie and forwards to PUT /v1/me/preferences;
    // backend failures are logged + swallowed so the cookie still wins
    // for this browser.
    await setTheme(raw, session.token);
    revalidatePath('/settings');
    redirect('/settings?status=theme_updated#theme');
  }

  async function waveSpeedAction(formData: FormData) {
    'use server';
    const session = await getSession();
    if (!session) redirect('/auth/login?next=/settings#theme');
    const raw = formData.get('wave_speed');
    if (!isWaveSpeed(raw)) {
      redirect('/settings?error=invalid_wave_speed#theme');
    }
    try {
      await putPreferences(session.token, { theme_wave_speed: raw });
    } catch (e) {
      logger.error({ err: e }, 'put preferences (wave speed) failed');
      redirect('/settings?error=unexpected#theme');
    }
    revalidatePath('/settings');
    redirect('/settings?status=wave_speed_updated#theme');
  }

  async function timezoneAction(formData: FormData) {
    'use server';
    const session = await getSession();
    if (!session) redirect('/auth/login?next=/settings#timezone');
    const raw = formData.get('timezone');
    // Shape check only — the API validates against the real tz database,
    // which is the authority. Duplicating a zone list here would rot.
    if (typeof raw !== 'string' || raw.length === 0 || raw.length > 64) {
      redirect('/settings?error=invalid_timezone#timezone');
    }
    try {
      await putPreferences(session.token, { timezone: raw as string });
    } catch (e) {
      logger.error({ err: e }, 'put preferences (timezone) failed');
      redirect('/settings?error=invalid_timezone#timezone');
    }
    revalidatePath('/settings');
    redirect('/settings?status=timezone_updated#timezone');
  }

  async function resendAction() {
    'use server';
    const session = await getSession();
    if (!session) redirect('/auth/login?next=/settings');
    try {
      await resendVerification(session.token);
    } catch (e) {
      if (e instanceof ApiCallError && e.status === 401) {
        redirect('/auth/login?next=/settings');
      }
      if (e instanceof ApiCallError && e.status === 409) {
        redirect('/settings?status=already_verified#verification');
      }
      logger.error({ err: e }, 'resend verification failed');
      redirect('/settings?error=unexpected#verification');
    }
    redirect('/settings?status=resent#verification');
  }

  async function rsiCheckAction() {
    'use server';
    const session = await getSession();
    if (!session) redirect('/auth/login?next=/settings');
    let verified = false;
    try {
      const resp = await rsiVerifyCheck(session.token);
      verified = resp.verified;
    } catch (e) {
      if (e instanceof ApiCallError) {
        if (e.status === 401) redirect('/auth/login?next=/settings');
        if (e.status === 422) redirect('/settings?error=rsi_code_not_in_bio#rsi');
        if (e.status === 404) redirect('/settings?error=rsi_handle_not_found#rsi');
        if (e.status === 410) redirect('/settings?error=rsi_code_expired#rsi');
        if (e.status === 503) redirect('/settings?error=rsi_unavailable#rsi');
      }
      logger.error({ err: e }, 'rsi verify check failed');
      redirect('/settings?error=unexpected#rsi');
    }
    redirect(
      verified
        ? '/settings?status=rsi_verified#rsi'
        : '/settings?error=rsi_unknown#rsi',
    );
  }

  async function refreshProfileAction() {
    'use server';
    const session = await getSession();
    if (!session) redirect('/auth/login?next=/settings');
    try {
      await refreshProfile(session.token);
    } catch (e) {
      if (e instanceof ApiCallError) {
        if (e.status === 401) redirect('/auth/login?next=/settings');
        if (e.status === 422) {
          redirect('/settings?error=rsi_handle_not_verified#rsi');
        }
        if (e.status === 429) redirect('/settings?error=refresh_too_soon#rsi');
        if (e.status === 404) {
          redirect('/settings?error=rsi_handle_not_found#rsi');
        }
        if (e.status === 503) redirect('/settings?error=rsi_unavailable#rsi');
      }
      logger.error({ err: e }, 'refresh profile failed');
      redirect('/settings?error=unexpected#rsi');
    }
    redirect('/settings?status=profile_refreshed#rsi');
  }

  async function refreshRsiOrgsAction() {
    'use server';
    const session = await getSession();
    if (!session) redirect('/auth/login?next=/settings');
    try {
      await refreshRsiOrgs(session.token);
    } catch (e) {
      if (e instanceof ApiCallError) {
        if (e.status === 401) redirect('/auth/login?next=/settings');
        if (e.status === 422) {
          redirect('/settings?error=rsi_handle_not_verified#rsi');
        }
        if (e.status === 429) {
          redirect('/settings?error=orgs_refresh_too_soon#rsi');
        }
        if (e.status === 404) {
          redirect('/settings?error=rsi_handle_not_found#rsi');
        }
        if (e.status === 503) redirect('/settings?error=rsi_unavailable#rsi');
      }
      logger.error({ err: e }, 'refresh rsi orgs failed');
      redirect('/settings?error=unexpected#rsi');
    }
    redirect('/settings?status=orgs_refreshed#rsi');
  }

  async function emailChangeAction(formData: FormData) {
    'use server';
    const session = await getSession();
    if (!session) redirect('/auth/login?next=/settings');
    const new_email = String(formData.get('new_email') ?? '').trim();
    if (new_email === '') {
      redirect('/settings?error=invalid_email#email');
    }
    try {
      await emailChangeStart(session.token, { new_email });
    } catch (e) {
      if (e instanceof ApiCallError) {
        if (e.status === 401) redirect('/auth/login?next=/settings');
        if (e.status === 409) redirect('/settings?error=email_taken#email');
        if (e.status === 400) {
          redirect(
            `/settings?error=${encodeURIComponent(e.body.error)}#email`,
          );
        }
      }
      logger.error({ err: e }, 'email change start failed');
      redirect('/settings?error=unexpected#email');
    }
    redirect('/settings?status=email_change_sent#email');
  }

  async function passwordAction(formData: FormData) {
    'use server';
    const session = await getSession();
    if (!session) redirect('/auth/login?next=/settings');

    const current_password = String(formData.get('current_password') ?? '');
    const new_password = String(formData.get('new_password') ?? '');

    try {
      await changePassword(session.token, { current_password, new_password });
    } catch (e) {
      if (e instanceof ApiCallError) {
        if (e.status === 401) {
          redirect('/settings?error=invalid_credentials#password');
        }
        if (e.status === 400) {
          redirect(
            `/settings?error=${encodeURIComponent(e.body.error)}#password`,
          );
        }
      }
      logger.error({ err: e }, 'change password failed');
      redirect('/settings?error=unexpected#password');
    }
    redirect('/settings?status=password_changed#password');
  }

  async function deleteAction(formData: FormData) {
    'use server';
    const session = await getSession();
    if (!session) redirect('/auth/login?next=/settings');

    const confirm_handle = String(formData.get('confirm_handle') ?? '').trim();

    try {
      await deleteAccount(session.token, { confirm_handle });
    } catch (e) {
      if (e instanceof ApiCallError) {
        if (e.status === 401) {
          redirect('/auth/login?next=/settings');
        }
        if (e.status === 400) {
          redirect(
            `/settings?error=${encodeURIComponent(e.body.error)}#danger`,
          );
        }
      }
      logger.error({ err: e }, 'delete account failed');
      redirect('/settings?error=unexpected#danger');
    }
    // Account is gone — drop the cookie and bounce to the marketing page.
    await clearSession();
    redirect('/');
  }

  // ---------------------------------------------------------------------
  // Sections.
  //
  // The ten anchored sections of the real page, each carrying the same server
  // action it always did. `id` values are LOAD-BEARING: every action above
  // redirects to one of these fragments, and `/settings/2fa` redirects to
  // `#security`. `group` maps a section onto the lens rail, and the rail is
  // driven from the fragment so those redirects still land — see
  // SettingsProjection.
  //
  // Groups and ids are the ones the scroll-spy sidebar used before the port
  // (its `settings-nav-config.ts` is gone with it); `_projection/groups.ts` is
  // now the group axis, and these ids remain the redirect targets.
  // ---------------------------------------------------------------------
  const sections: SettingsSection[] = [
    {
      id: 'retention',
      title: 'Retention',
      ctx: 'What is stored, and for how long',
      group: 'general',
      node: (
        <>
          {/* `Calibrate.jsx` carries a Retention pane and this page had none —
              so the single most important fact about a reader's data, that it
              is bounded at a year, was surfaced nowhere in settings.

              THE SPEC'S OTHER ROWS ARE OMITTED, NOT FAKED. Its pane also lists
              parsed-event bytes, a raw-archive size, a free-tier cap and an
              export format, with "Export manifest" and "Re-parse local store"
              buttons. The product has no export endpoint and no storage
              accounting — inventing figures for them here would be exactly the
              inferred-field trap the kit's own Unverified banner warns about,
              on the page where a reader goes to find out what is kept. */}
          <HoloKV
            items={[
              { k: 'Window', v: '365 days (server maximum)' },
              { k: 'Beyond the window', v: 'Deleted, not archived' },
            ]}
          />
          <p className="hp-prose">
            Nothing older than a year is kept, which is why the widest range is
            called All rather than all time — it is everything there is, not
            everything there ever was.
          </p>
        </>
      ),
    },
    {
      id: 'theme',
      title: 'Emitter calibration',
      ctx: 'Repaints every projection',
      group: 'general',
      // The picker is a CLIENT control: it recalibrates in place (repaints the
      // beam and fires the shock) instead of posting and reloading, which the
      // flat `ThemeSwatchGrid` also did via its wave. The projection supplies
      // it; `themeAction` below is the no-JS fallback.
      slot: 'calibration',
      node: (
        <>
          <p className="hp-prose">
            Calibrations change the beam. Type, spacing and component shapes
            are identical across all four. Your choice follows you across
            devices.
          </p>
          <Plane tilt="flat" cap="Wave speed" style={{ marginTop: 22 }}>
            <p className="hp-prose" style={{ marginTop: 0 }}>
              How fast the sweep runs when you recalibrate. &quot;Off&quot;
              swaps instantly with no animation.
            </p>
            <WaveSpeedField
              active={activeWaveSpeed}
              waveSpeedAction={waveSpeedAction}
            />
          </Plane>
        </>
      ),
    },
    {
      id: 'timezone',
      title: 'Local time',
      ctx: 'Used for facts about when you fly',
      group: 'general',
      node: (
        <>
          <p className="hp-prose">
            Used for facts about when you fly. Without it, times of day are
            left out entirely.
          </p>
          <TimezoneField
            storedTimezone={storedTimezone}
            timezoneAction={timezoneAction}
          />
        </>
      ),
    },
    {
      id: 'account-info',
      title: 'Account info',
      group: 'account',
      node: (
        <div style={{ marginTop: 16 }}>
          <HoloKV
            items={[
              {
                k: 'Email',
                v: (
                  <>
                    {me.email}{' '}
                    <BeamChip tone={me.email_verified ? 'good' : 'warn'} dot>
                      {me.email_verified ? 'Verified' : 'Not verified'}
                    </BeamChip>
                  </>
                ),
              },
              {
                k: 'RSI handle',
                v: (
                  <>
                    {me.claimed_handle}{' '}
                    <BeamChip tone={me.rsi_verified ? 'good' : 'warn'} dot>
                      {me.rsi_verified ? 'Ownership proven' : 'Unverified'}
                    </BeamChip>
                  </>
                ),
              },
              ...(me.pending_email
                ? [
                    {
                      k: 'Pending email',
                      v: `${me.pending_email} · awaiting confirmation`,
                    },
                  ]
                : []),
            ]}
          />
        </div>
      ),
    },
    {
      id: 'verification',
      title: 'Email verification',
      group: 'account',
      node: me.email_verified ? (
        <p className="hp-prose">Your email is verified. Nothing to do here.</p>
      ) : (
        <>
          <p className="hp-prose">
            We sent a verification link to{' '}
            <span className="val">{me.email}</span>. Didn&apos;t arrive?
            Resend it below.
          </p>
          <form action={resendAction} className="hp-formcol">
            <BeamButton
              type="submit"
              variant="primary"
              style={{ alignSelf: 'flex-start' }}
            >
              Resend verification link
            </BeamButton>
          </form>
        </>
      ),
    },
    {
      id: 'rsi',
      title: 'RSI handle ownership',
      ctx: me.rsi_verified ? 'Proven' : 'Unproven',
      group: 'account',
      node: me.rsi_verified ? (
        <>
          <p className="hp-prose">
            <span className="val">{me.claimed_handle}</span> is verified.
            Sharing, org access and your public profile are unlocked.
          </p>
          <Plane
            tilt="flat"
            cap="Citizen profile snapshot"
            style={{ marginTop: 20 }}
          >
            {profileLoadFailed ? (
              <p className="hp-prose" style={{ marginTop: 0 }}>
                Couldn&apos;t load the snapshot. Refresh the page or try again.
              </p>
            ) : profile ? (
              <p className="hp-prose" style={{ marginTop: 0 }}>
                Last refreshed:{' '}
                <span className="val">
                  {new Date(profile.captured_at).toLocaleString()}
                </span>
                .
              </p>
            ) : (
              <p className="hp-prose" style={{ marginTop: 0 }}>
                You haven&apos;t snapshotted your RSI profile yet. Snapshots
                cache your display name, badges, bio and primary org so they
                show up on your projection and public profile.
              </p>
            )}
            <div
              style={{
                display: 'flex',
                flexWrap: 'wrap',
                gap: 10,
                marginTop: 16,
              }}
            >
              <form action={refreshProfileAction}>
                <BeamButton type="submit" variant="ghost">
                  Refresh profile
                </BeamButton>
              </form>
              <form action={refreshRsiOrgsAction}>
                <BeamButton type="submit" variant="ghost">
                  Refresh orgs
                </BeamButton>
              </form>
            </div>
          </Plane>
        </>
      ) : rsiLoadFailed ? (
        <p className="hp-prose">
          Couldn&apos;t load the verification code right now. Refresh the page
          to try again.
        </p>
      ) : rsiState ? (
        <>
          <p className="hp-prose">
            Public profiles and shares display{' '}
            <span className="val">{me.claimed_handle}</span> as your name. To
            stop someone signing up as a handle that isn&apos;t theirs, we ask
            you to prove ownership by pasting a short code into your RSI public
            bio. Once verified, you can take the code back out — we only check
            it once.
          </p>
          <ol className="hp-steps">
            <li>
              Open{' '}
              <a
                href={`https://robertsspaceindustries.com/citizens/${encodeURIComponent(me.claimed_handle)}`}
                target="_blank"
                rel="noopener noreferrer"
              >
                your RSI public profile
              </a>{' '}
              and click <em>Edit Profile</em> → <em>Bio</em>.
            </li>
            <li>
              Paste this code anywhere in the bio:
              <div className="hp-secret">{rsiState.code}</div>
              <p className="hp-prose" style={{ marginTop: 8 }}>
                Expires{' '}
                <span className="val">
                  {rsiState.expires_at
                    ? new Date(rsiState.expires_at).toLocaleString()
                    : '(unknown)'}
                </span>
                . Save the bio in RSI before pressing the button below.
              </p>
            </li>
            <li>
              <form action={rsiCheckAction} className="hp-formcol">
                <BeamButton
                  type="submit"
                  variant="primary"
                  style={{ alignSelf: 'flex-start' }}
                >
                  Check now
                </BeamButton>
              </form>
            </li>
          </ol>
        </>
      ) : (
        <p className="hp-prose">Loading verification state…</p>
      ),
    },
    {
      id: 'hangar',
      title: 'Device sync',
      ctx: 'Written by the tray, read here',
      group: 'account',
      node: <HangarPane snapshot={hangar} />,
    },
    {
      id: 'email',
      title: 'Change sign-in email',
      group: 'account',
      node: (
        <>
          {me.pending_email ? (
            <p className="hp-prose">
              We sent a confirmation link to{' '}
              <span className="val">{me.pending_email}</span>. Click it from
              that inbox to switch your sign-in email. The link expires in 24
              hours. Submitting this form again replaces the pending address.
            </p>
          ) : (
            <p className="hp-prose">
              We&apos;ll send a confirmation link to the new address; your
              sign-in email only changes after you click it. Your current
              address (<span className="val">{me.email}</span>) stays active
              until then.
            </p>
          )}
          <form action={emailChangeAction} className="hp-formrow">
            <BeamInput
              id="new-email"
              label="New email"
              type="email"
              name="new_email"
              required
              autoComplete="email"
              spellCheck={false}
              placeholder="new@example.com"
            />
            <BeamButton type="submit" variant="primary">
              {me.pending_email
                ? 'Replace pending change'
                : 'Send confirmation link'}
            </BeamButton>
          </form>
        </>
      ),
    },
    {
      id: 'password',
      title: 'Change password',
      group: 'security',
      node: (
        <form action={passwordAction} className="hp-formcol">
          <BeamInput
            id="current-password"
            label="Current password"
            type="password"
            name="current_password"
            required
            autoComplete="current-password"
          />
          <BeamInput
            id="new-password"
            label="New password"
            type="password"
            name="new_password"
            required
            minLength={12}
            autoComplete="new-password"
            hint="At least 12 characters."
          />
          <BeamButton
            type="submit"
            variant="primary"
            style={{ alignSelf: 'flex-start' }}
          >
            Update password
          </BeamButton>
        </form>
      ),
    },
    {
      id: 'security',
      title: 'Two-factor authentication',
      group: 'security',
      // The wizard's state machine is untouched by the port — only its
      // rendering moved into the beam. See SecuritySection.
      node: <SecuritySection me={me} />,
    },
    {
      id: 'danger',
      title: 'Delete account',
      ctx: 'No undo',
      group: 'danger',
      node: (
        <form id="delete-account-form" action={deleteAction}>
          <p className="hp-prose">
            Deleting your account is permanent. Your account record, paired
            devices and active shares are removed. Your ingested game events
            are pseudonymised — the row count is preserved so anyone you shared
            with keeps a coherent timeline, but the data is no longer linked to
            you or your RSI handle. To confirm, type your RSI handle (
            <span className="val">{me.claimed_handle}</span>) below.
          </p>
          <div className="hp-formrow">
            <BeamInput
              id="confirm-handle"
              label="Type your handle to confirm"
              type="text"
              name="confirm_handle"
              required
              autoComplete="off"
              spellCheck={false}
              placeholder={me.claimed_handle}
            />
            <BeamButton type="submit" variant="danger">
              Delete my account
            </BeamButton>
          </div>
        </form>
      ),
    },
  ];

  // `?status=` is a success and `?error=` is a failure; both map through the
  // same label tables the flat page used, so the copy is unchanged.
  const notice = status
    ? { tone: 'good' as const, message: labelForStatus(status) }
    : error
      ? { tone: 'bad' as const, message: labelForError(error) }
      : null;

  return (
    <SettingsProjection
      handle={session.claimedHandle}
      calibration={activeTheme as Calibration}
      nav={navSections({ signedIn: true, staffRoles: session.staffRoles }, 'settings')}
      groups={SETTINGS_GROUPS}
      sections={sections}
      notice={notice}
      themeAction={themeAction}
      onCalibrate={async (id: string) => {
        'use server';
        await setCalibrationAction(id);
      }}
    />
  );
}

function labelForStatus(code: string): string {
  switch (code) {
    case 'resent':
      return 'Verification email sent. Check your inbox.';
    case 'already_verified':
      return 'Your email is already verified — no message was sent.';
    case 'password_changed':
      return 'Password updated.';
    case 'visibility_public':
      return 'Your profile is now public.';
    case 'visibility_private':
      return 'Your profile is now private.';
    case 'share_added':
      return 'Access granted.';
    case 'share_revoked':
      return 'Access revoked.';
    case 'org_share_added':
      return 'Org access granted.';
    case 'org_share_revoked':
      return 'Org access revoked.';
    case 'email_change_sent':
      return 'Confirmation link sent. Check the new inbox to finish the change.';
    case 'rsi_verified':
      return 'RSI handle verified. You can take the code back out of your bio now.';
    case 'profile_refreshed':
      return 'Profile snapshot refreshed.';
    case 'orgs_refreshed':
      return 'Org snapshot refreshed.';
    case 'theme_updated':
      return 'Theme updated.';
    case 'wave_speed_updated':
      return 'Wave speed updated.';
    case 'totp_enabled':
      return "Two-factor enabled. Save your recovery codes below — you won't see them again.";
    case 'totp_ack':
      return 'Recovery codes acknowledged.';
    case 'totp_disabled':
      return 'Two-factor disabled.';
    case 'totp_regenerated':
      return 'New recovery codes generated. Save them — the old set is gone.';
    default:
      return 'Done.';
  }
}

function labelForError(code: string): string {
  switch (code) {
    case 'invalid_credentials':
      return 'Current password is incorrect.';
    case 'password_too_short':
      return 'New password must be at least 12 characters.';
    case 'confirm_mismatch':
      return "That handle doesn't match. Account was not deleted.";
    case 'recipient_not_found':
      return "We couldn't find a StarStats user with that handle.";
    case 'cannot_share_with_self':
      return "You can't share your stats with yourself.";
    case 'invalid_recipient_handle':
      return 'That handle looks invalid. Use letters, digits, _ or -.';
    case 'invalid_org_slug':
      return 'That org slug looks invalid.';
    case 'org_not_found':
      return "We couldn't find an org with that slug.";
    case 'spicedb_unavailable':
      return 'Sharing is temporarily unavailable. Please try again shortly.';
    case 'rsi_handle_not_verified':
      return "Verify your RSI handle (above) before sharing — public profiles and shares display your handle, so we need to confirm it's yours.";
    case 'invalid_email':
      return 'That email address looks invalid.';
    case 'email_taken':
      return 'That email is already in use by another account.';
    case 'rsi_code_not_in_bio':
      return "We couldn't find the code in your RSI bio. Make sure you saved the bio after pasting it.";
    case 'rsi_handle_not_found':
      return "RSI doesn't have a public profile for that handle. Check the spelling matches your RSI account exactly.";
    case 'rsi_code_expired':
      return 'The verification code expired. Refresh this page to get a fresh one.';
    case 'rsi_unavailable':
      return 'RSI is unreachable right now. Please try again in a few minutes.';
    case 'rsi_unknown':
      return 'Something went wrong checking your bio. Please try again.';
    case 'refresh_too_soon':
      return 'Profile was just refreshed — please wait a few minutes before refreshing again.';
    case 'orgs_refresh_too_soon':
      return 'Orgs were just refreshed — please wait a few minutes before refreshing again.';
    case 'invalid_theme':
      return "That theme isn't recognised. Pick one of the four shown.";
    case 'invalid_wave_speed':
      return "That wave speed isn't recognised. Pick one of the four shown.";
    case 'invalid_code':
      return "That authentication code didn't match. Check the time on your device and try again.";
    case 'no_setup':
      return 'Start two-factor setup before trying to confirm.';
    case 'already_enabled':
      return 'Two-factor is already enabled on this account.';
    case 'not_enabled':
      return "Two-factor isn't enabled on this account.";
    case 'unexpected':
      return 'Something went wrong. Please try again.';
    default:
      return `Couldn't complete that action (${code}).`;
  }
}
