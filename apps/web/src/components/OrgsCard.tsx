import type { RsiOrgsSnapshot } from '@/lib/api';
import { refreshOrgsAction } from '@/app/_actions/refresh-rsi';
import { RefreshSubmitButton } from '@/components/cards/RefreshSubmitButton';

/**
 * RSI org-membership card. Shared between:
 *   * the dashboard (private snapshot via `getMyRsiOrgs`), and
 *   * the public profile page `/u/[handle]` (public snapshot via
 *     `getPublicRsiOrgs`).
 *
 * The two server endpoints return the same `RsiOrgsSnapshot` shape, so
 * one component covers both. The owner-side path passes
 * `showSettingsLink` so the empty state CTA points at the refresh
 * button on /settings; the public-side path omits it because a
 * stranger can't refresh someone else's snapshot.
 *
 * `OrgsCardInner` renders only the content (org list or empty-state
 * body) with NO outer <section>, eyebrow, or h2. It is used by the
 * profile widget system (WidgetFrame already supplies that chrome).
 * `OrgsCard` wraps `OrgsCardInner` with the full card shell and is
 * the public API consumed by legacy (flag-off) call sites.
 */

function formatRelative(iso: string): string {
  const then = new Date(iso).getTime();
  if (Number.isNaN(then)) return iso;
  const deltaSec = Math.round((then - Date.now()) / 1000);
  const rtf = new Intl.RelativeTimeFormat('en', { numeric: 'auto' });
  const abs = Math.abs(deltaSec);
  if (abs < 60) return rtf.format(deltaSec, 'second');
  if (abs < 3600) return rtf.format(Math.round(deltaSec / 60), 'minute');
  if (abs < 86_400) return rtf.format(Math.round(deltaSec / 3600), 'hour');
  if (abs < 30 * 86_400) {
    return rtf.format(Math.round(deltaSec / 86_400), 'day');
  }
  if (abs < 365 * 86_400) {
    return rtf.format(Math.round(deltaSec / (30 * 86_400)), 'month');
  }
  return rtf.format(Math.round(deltaSec / (365 * 86_400)), 'year');
}

/**
 * Inner content only — no <section> wrapper, no eyebrow, no outer h2.
 * Used by the orgs widget so WidgetFrame supplies the card chrome once.
 */
export function OrgsCardInner({
  snapshot,
}: {
  snapshot: RsiOrgsSnapshot | null;
}) {
  if (!snapshot) {
    return (
      <>
        <h2
          style={{
            margin: 0,
            fontSize: 17,
            fontWeight: 600,
            letterSpacing: '-0.01em',
          }}
        >
          No org snapshot yet
        </h2>
        <p
          style={{
            margin: '8px 0 0',
            color: 'var(--fg-muted)',
            fontSize: 13,
            lineHeight: 1.55,
          }}
        >
          This citizen hasn&apos;t published their org memberships.
        </p>
      </>
    );
  }

  // Move the main org first so it leads. Stable sort against the rest
  // by name so renders are consistent across reloads.
  const sorted = [...snapshot.orgs].sort((a, b) => {
    if (a.is_main !== b.is_main) return a.is_main ? -1 : 1;
    return a.name.localeCompare(b.name);
  });

  return (
    <>
      {/*
        Audit v2 §08 — promote refresh to an inline ghost button for the
        owner-side render. Public callers omit `showSettingsLink`, so
        strangers don't see it.
      */}
      <div
        style={{
          display: 'flex',
          justifyContent: 'space-between',
          alignItems: 'center',
          gap: 12,
          marginBottom: 4,
        }}
      >
        <h2
          style={{
            margin: 0,
            fontSize: 17,
            fontWeight: 600,
            letterSpacing: '-0.01em',
          }}
        >
          {sorted.length === 0 ? 'No public memberships' : 'Memberships'}
        </h2>
      </div>
      <p
        style={{
          margin: 0,
          color: 'var(--fg-dim)',
          fontSize: 12,
          marginBottom: 14,
        }}
      >
        Snapshot {formatRelative(snapshot.captured_at)}
      </p>

      {sorted.length === 0 ? (
        <p style={{ margin: 0, color: 'var(--fg-muted)', fontSize: 13 }}>
          No orgs were visible on the citizen card at capture time. RSI
          hides redacted memberships from public scrapes.
        </p>
      ) : (
        <ul
          style={{
            listStyle: 'none',
            margin: 0,
            padding: 0,
            display: 'flex',
            flexDirection: 'column',
            gap: 8,
            fontSize: 13,
          }}
        >
          {sorted.map((org) => (
            <li
              key={org.sid}
              style={{
                display: 'flex',
                justifyContent: 'space-between',
                alignItems: 'baseline',
                gap: 12,
              }}
            >
              <span style={{ display: 'inline-flex', gap: 8, alignItems: 'baseline' }}>
                <span style={{ color: 'var(--fg)' }}>{org.name}</span>
                {org.is_main && (
                  <span
                    className="ss-badge"
                    style={{ fontSize: 10, padding: '2px 6px' }}
                  >
                    Main
                  </span>
                )}
              </span>
              <span
                className="mono"
                style={{ color: 'var(--fg-muted)', fontSize: 12 }}
              >
                {org.rank ?? 'rank hidden'} · {org.sid}
              </span>
            </li>
          ))}
        </ul>
      )}
    </>
  );
}

export function OrgsCard({
  snapshot,
  showSettingsLink = false,
}: {
  snapshot: RsiOrgsSnapshot | null;
  showSettingsLink?: boolean;
}) {
  if (!snapshot) {
    return (
      <section className="ss-card ss-card-pad">
        <div className="ss-eyebrow" style={{ marginBottom: 6 }}>
          RSI organisations
        </div>
        <h2
          style={{
            margin: 0,
            fontSize: 17,
            fontWeight: 600,
            letterSpacing: '-0.01em',
          }}
        >
          No org snapshot yet
        </h2>
        <p
          style={{
            margin: '8px 0 0',
            color: 'var(--fg-muted)',
            fontSize: 13,
            lineHeight: 1.55,
          }}
        >
          {showSettingsLink ? (
            <>
              Pull a fresh org list from RSI.{' '}
              <a href="/settings#rsi" style={{ color: 'var(--accent)' }}>
                Refresh from Settings
              </a>
              .
            </>
          ) : (
            "This citizen hasn't published their org memberships."
          )}
        </p>
      </section>
    );
  }

  // Move the main org first so it leads. Stable sort against the rest
  // by name so renders are consistent across reloads.
  const sorted = [...snapshot.orgs].sort((a, b) => {
    if (a.is_main !== b.is_main) return a.is_main ? -1 : 1;
    return a.name.localeCompare(b.name);
  });

  return (
    <section className="ss-card ss-card-pad">
      <div className="ss-eyebrow" style={{ marginBottom: 6 }}>
        RSI organisations
      </div>
      {/*
        Audit v2 §08 — promote refresh to an inline ghost button for the
        owner-side render. Public callers omit `showSettingsLink`, so
        strangers don't see it.
      */}
      <div
        style={{
          display: 'flex',
          justifyContent: 'space-between',
          alignItems: 'center',
          gap: 12,
          marginBottom: 4,
        }}
      >
        <h2
          style={{
            margin: 0,
            fontSize: 17,
            fontWeight: 600,
            letterSpacing: '-0.01em',
          }}
        >
          {sorted.length === 0 ? 'No public memberships' : 'Memberships'}
        </h2>
        {showSettingsLink && (
          <form action={refreshOrgsAction}>
            <RefreshSubmitButton />
          </form>
        )}
      </div>
      <p
        style={{
          margin: 0,
          color: 'var(--fg-dim)',
          fontSize: 12,
          marginBottom: 14,
        }}
      >
        Snapshot {formatRelative(snapshot.captured_at)}
      </p>

      {sorted.length === 0 ? (
        <p style={{ margin: 0, color: 'var(--fg-muted)', fontSize: 13 }}>
          No orgs were visible on the citizen card at capture time. RSI
          hides redacted memberships from public scrapes.
        </p>
      ) : (
        <ul
          style={{
            listStyle: 'none',
            margin: 0,
            padding: 0,
            display: 'flex',
            flexDirection: 'column',
            gap: 8,
            fontSize: 13,
          }}
        >
          {sorted.map((org) => (
            <li
              key={org.sid}
              style={{
                display: 'flex',
                justifyContent: 'space-between',
                alignItems: 'baseline',
                gap: 12,
              }}
            >
              <span style={{ display: 'inline-flex', gap: 8, alignItems: 'baseline' }}>
                <span style={{ color: 'var(--fg)' }}>{org.name}</span>
                {org.is_main && (
                  <span
                    className="ss-badge"
                    style={{ fontSize: 10, padding: '2px 6px' }}
                  >
                    Main
                  </span>
                )}
              </span>
              <span
                className="mono"
                style={{ color: 'var(--fg-muted)', fontSize: 12 }}
              >
                {org.rank ?? 'rank hidden'} · {org.sid}
              </span>
            </li>
          ))}
        </ul>
      )}
    </section>
  );
}
