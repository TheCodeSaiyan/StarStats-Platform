/**
 * /settings/widget-sharing — Per-widget visitor visibility toggles.
 *
 * Plan 3b Option A: owners opt each widget in or out of visitor visibility.
 * Changes are applied via the `saveShareScopesAction` server action, which
 * PUTs to `/v1/users/me/share-scopes` and revalidates the profile page.
 *
 * All five toggles default to OFF (private). The owner's profile page
 * checks `shareScopes` before rendering each widget for visitors.
 */

import { redirect } from 'next/navigation';
import { getSession } from '@/lib/session';
import { getMyShareScopes, ApiCallError, type WidgetShareScopesApi } from '@/lib/api';
import { logger } from '@/lib/logger';
import { saveShareScopesAction } from '@/app/_actions/share-scopes';
import { SHARE_SCOPES, type ShareScopeMeta } from '@/lib/share-scopes';

export const metadata = { title: "Widget sharing" };

// -- Style helpers (match settings/page.tsx conventions) ------------------

const pageStyle: React.CSSProperties = {
  display: 'flex',
  flexDirection: 'column',
  gap: 20,
  maxWidth: 960,
  margin: '0 auto',
  padding: '8px 0 60px',
};

const cardHeaderStyle: React.CSSProperties = {
  padding: '20px 24px 0',
};

const cardBodyStyle: React.CSSProperties = {
  padding: '16px 24px 22px',
  display: 'flex',
  flexDirection: 'column',
  gap: 14,
};

const cardFooterStyle: React.CSSProperties = {
  padding: '14px 24px',
  borderTop: '1px solid var(--border)',
};

// -- Widget metadata -------------------------------------------------------

/**
 * MOVED to `lib/share-scopes.ts`. The public profile states what a pilot
 * publishes and what they withhold, and needed these same five labels; a
 * second copy would have drifted from this one the moment it existed.
 */
const WIDGET_LABELS: readonly ShareScopeMeta[] = SHARE_SCOPES;

// -- Page component --------------------------------------------------------

interface SearchParams {
  status?: string;
  error?: string;
}

export default async function WidgetSharingPage({
  searchParams,
}: {
  searchParams: Promise<SearchParams>;
}) {
  const session = await getSession();
  if (!session) {
    redirect('/auth/login?next=/settings/widget-sharing');
  }

  const params = await searchParams;

  // Fetch current scopes. On failure degrade to all-false — the user
  // can still see the form and submit (which will set explicit values).
  let scopes: WidgetShareScopesApi = {
    combat_mission: false,
    economy: false,
    travel: false,
    records: false,
    recent_activity: false,
  };
  let loadFailed = false;
  try {
    scopes = await getMyShareScopes(session.token);
  } catch (e) {
    if (e instanceof ApiCallError && e.status === 401) {
      redirect('/auth/login?next=/settings/widget-sharing');
    }
    logger.warn({ err: e }, 'load share scopes failed');
    loadFailed = true;
  }

  // -- Server action -------------------------------------------------------

  async function saveAction(formData: FormData) {
    'use server';
    // Derive keys from WIDGET_LABELS so adding a new widget only requires
    // updating WIDGET_LABELS — this action picks it up automatically.
    const newScopes = Object.fromEntries(
      WIDGET_LABELS.map(({ key }) => [key, formData.get(key) === 'on']),
    ) as WidgetShareScopesApi;
    const result = await saveShareScopesAction(newScopes);
    if (result.ok) {
      redirect('/settings/widget-sharing?status=saved');
    } else {
      redirect('/settings/widget-sharing?error=save_failed');
    }
  }

  // -- Render ---------------------------------------------------------------

  return (
    <div style={pageStyle}>
      <header>
        <div className="ss-eyebrow" style={{ marginBottom: 8 }}>
          Sharing settings
        </div>
        <h1 className="hp-pagetitle">Widget visibility</h1>
        <p className="hp-recsub">
          Choose which widgets visitors can see on your profile. All widgets
          default to private — toggle one on to share it.
        </p>
      </header>

      {params.status === 'saved' && (
        <div role="status" className="ss-alert ss-alert--ok">
          Widget sharing preferences saved.
        </div>
      )}

      {(params.error || loadFailed) && (
        <div role="alert" className="ss-alert ss-alert--danger">
          {loadFailed
            ? 'Could not load current settings — the form shows defaults. Your saved preferences are unaffected.'
            : params.error === 'save_failed'
              ? 'Save failed — please try again.'
              : 'An unexpected error occurred.'}
        </div>
      )}

      <div className="ss-card">
        <div style={cardHeaderStyle}>
          <h2
            style={{
              margin: 0,
              fontSize: 17,
              fontWeight: 600,
              letterSpacing: '-0.01em',
            }}
          >
            Per-widget toggles
          </h2>
          <p
            style={{
              margin: '4px 0 0',
              color: 'var(--fg-muted)',
              fontSize: 13,
            }}
          >
            These toggles only apply to visitors who already have access to
            your profile (via public mode or a direct share). Turning a widget
            on here does not make your profile public.
          </p>
        </div>

        <form action={saveAction}>
          <div style={cardBodyStyle}>
            {WIDGET_LABELS.map(({ key, label, description }) => (
              <label
                key={key}
                style={{
                  display: 'flex',
                  alignItems: 'flex-start',
                  gap: 14,
                  cursor: 'pointer',
                  padding: '10px 0',
                  borderBottom: '1px solid var(--border)',
                }}
              >
                <input
                  type="checkbox"
                  name={key}
                  defaultChecked={scopes[key]}
                  style={{
                    marginTop: 2,
                    width: 16,
                    height: 16,
                    flexShrink: 0,
                    accentColor: 'var(--accent)',
                  }}
                />
                <div>
                  <div
                    style={{
                      fontSize: 14,
                      fontWeight: 500,
                      color: 'var(--fg)',
                    }}
                  >
                    {label}
                  </div>
                  <div
                    style={{
                      fontSize: 12,
                      color: 'var(--fg-muted)',
                      marginTop: 2,
                    }}
                  >
                    {description}
                  </div>
                </div>
              </label>
            ))}
          </div>

          <div style={cardFooterStyle}>
            <button type="submit" className="ss-btn ss-btn--primary">
              Save preferences
            </button>
          </div>
        </form>
      </div>

      <div
        style={{
          padding: '14px 18px',
          background: 'var(--bg-elev)',
          border: '1px solid var(--border)',
          borderRadius: 0,
          color: 'var(--fg-dim)',
          fontSize: 12,
          lineHeight: 1.5,
        }}
      >
        <strong style={{ color: 'var(--fg-muted)' }}>How this works:</strong>{' '}
        Three widgets (Hangar, Loadout, and Entities) are always private and
        cannot be toggled here. Sessions, Heatmap, and Orgs follow your main
        sharing and public-visibility settings.
      </div>
    </div>
  );
}
