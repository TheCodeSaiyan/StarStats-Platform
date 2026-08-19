import Link from 'next/link';
import type { ProfileResponse } from '@/lib/api';
import { refreshProfileAction } from '@/app/_actions/refresh-rsi';
import { RefreshSubmitButton } from '@/components/cards/RefreshSubmitButton';

const BIO_MAX_CHARS = 600;

function truncateBio(bio: string): string {
  if (bio.length <= BIO_MAX_CHARS) return bio;
  return bio.slice(0, BIO_MAX_CHARS).trimEnd() + '…';
}

// RSI publishes the enlistment date without a timezone, so parse the
// `YYYY-MM-DD` components by hand — going through `Date` would let the
// viewer's local TZ shift the day boundary.
function formatEnlistment(raw: string): string {
  const m = raw.match(/^(\d{4})-(\d{2})-(\d{2})$/);
  if (!m) return raw;
  const year = Number(m[1]);
  const month = Number(m[2]) - 1;
  const day = Number(m[3]);
  const months = [
    'Jan', 'Feb', 'Mar', 'Apr', 'May', 'Jun',
    'Jul', 'Aug', 'Sep', 'Oct', 'Nov', 'Dec',
  ];
  if (month < 0 || month > 11) return raw;
  return months[month] + ' ' + day + ', ' + year;
}

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
 * RSI citizen profile snapshot card. The owner-facing call site passes
 * `showSettingsLink` so the empty state points back at /settings; the
 * public profile call site omits it because a stranger can't refresh
 * someone else's snapshot.
 */
export function ProfileCard({
  profile,
  showSettingsLink = false,
}: {
  profile: ProfileResponse | null;
  showSettingsLink?: boolean;
}) {
  if (!profile) {
    return (
      <section className="ss-card ss-card-pad">
        <div className="ss-eyebrow" style={{ marginBottom: 6 }}>
          RSI citizen profile
        </div>
        <h2
          style={{
            margin: 0,
            fontSize: 17,
            fontWeight: 600,
            letterSpacing: '-0.01em',
          }}
        >
          No snapshot yet
        </h2>
        <p style={{ margin: '8px 0 0', color: 'var(--fg-muted)', fontSize: 13 }}>
          Pull a fresh manifest from RSI.{' '}
          {showSettingsLink ? (
            <>
              <Link
                href="/settings#rsi"
                style={{ color: 'var(--accent)' }}
              >
                Refresh from Settings
              </Link>
              .
            </>
          ) : (
            'Refresh from Settings.'
          )}
        </p>
      </section>
    );
  }

  const rows: Array<[string, string]> = [];
  if (profile.display_name) rows.push(['Display name', profile.display_name]);
  if (profile.enlistment_date) {
    rows.push(['Enlisted', formatEnlistment(profile.enlistment_date)]);
  }
  if (profile.location) rows.push(['Location', profile.location]);
  if (profile.primary_org_summary) {
    rows.push(['Primary org', profile.primary_org_summary]);
  }
  rows.push(['Snapshot', formatRelative(profile.captured_at)]);

  return (
    <section className="ss-card ss-card-pad">
      <div className="ss-eyebrow" style={{ marginBottom: 6 }}>
        RSI citizen profile
      </div>
      {/*
        Audit v2 §08 promoted the refresh CTA to an inline button when
        the user owns the snapshot. Public-profile callers omit
        `showSettingsLink`, so strangers don't see it.
      */}
      <div
        style={{
          display: 'flex',
          justifyContent: 'space-between',
          alignItems: 'center',
          gap: 12,
          marginBottom: 16,
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
          Identity card
        </h2>
        {showSettingsLink && (
          <form action={refreshProfileAction}>
            <RefreshSubmitButton />
          </form>
        )}
      </div>

      {rows.length > 0 && (
        <dl
          className="ss-kv"
          style={{ gridTemplateColumns: '160px 1fr', marginBottom: profile.badges.length > 0 || profile.bio ? 18 : 0 }}
        >
          {rows.flatMap(([label, value]) => [
            <dt key={label + ':dt'}>{label}</dt>,
            <dd key={label + ':dd'} className="mono">
              {value}
            </dd>,
          ])}
        </dl>
      )}

      {profile.badges.length > 0 && (
        <div
          role="list"
          style={{
            display: 'flex',
            flexWrap: 'wrap',
            gap: 8,
            marginBottom: profile.bio ? 18 : 0,
          }}
        >
          {profile.badges.map((badge, idx) => (
            <span
              key={badge.name + '-' + idx}
              role="listitem"
              className="ss-badge"
              title={badge.name}
              style={{
                gap: 6,
                paddingRight: badge.image_url ? 10 : 8,
              }}
            >
              {badge.image_url && (
                <img
                  src={badge.image_url}
                  alt=""
                  aria-hidden="true"
                  style={{
                    width: 16,
                    height: 16,
                    objectFit: 'contain',
                    display: 'block',
                  }}
                />
              )}
              <span style={{ color: 'var(--fg)' }}>{badge.name}</span>
            </span>
          ))}
        </div>
      )}

      {profile.bio && (
        <p
          style={{
            margin: 0,
            color: 'var(--fg-muted)',
            fontSize: 13,
            lineHeight: 1.6,
            whiteSpace: 'pre-wrap',
          }}
        >
          {truncateBio(profile.bio)}
        </p>
      )}
    </section>
  );
}
