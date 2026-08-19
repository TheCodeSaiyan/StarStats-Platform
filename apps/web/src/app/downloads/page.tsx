import type { Metadata } from 'next';
import { fetchTrayReleases } from '@/lib/github-releases.server';
import type { TrayReleaseSet } from '@/lib/github-releases';
import { DownloadsView } from './DownloadsView';

export const metadata: Metadata = {
  title: 'Download',
  description:
    'Download the StarStats tray client — the local app that reads your Star Citizen Game.log. Windows and Linux builds, pulled live from the latest release.',
};

export default async function DownloadsPage() {
  let set: TrayReleaseSet = { stable: null, prerelease: null };
  let error = false;
  try {
    set = await fetchTrayReleases();
  } catch (err) {
    // Log the real cause (GitHub outage / rate-limit / bad shape) so a
    // "Couldn't reach the release feed" state is diagnosable server-side —
    // mirrors the fetcher pattern in lib/reference.ts.
    console.error('tray releases fetch failed', err);
    error = true;
  }

  return (
    <main style={{ maxWidth: 880, margin: '0 auto', padding: '48px 24px' }}>
      <header style={{ marginBottom: 8 }}>
        <span className="ss-placard" style={{ color: 'var(--fg-dim)' }}>
          Download
        </span>
        <h1
          style={{
            margin: '12px 0 12px',
            fontSize: 'clamp(40px, 6vw, 64px)',
            fontWeight: 600,
          }}
        >
          Get the StarStats tray
        </h1>
        <p style={{ margin: 0, color: 'var(--fg-dim)', maxWidth: '60ch', lineHeight: 1.6 }}>
          A small desktop app that reads what Star Citizen already writes to its{' '}
          <code>Game.log</code> — sessions, travel, loadouts — and keeps it on
          your machine until you sign in and turn on sync. Pick your platform
          below; the tray keeps itself up to date after that.
        </p>
      </header>

      <DownloadsView stable={set.stable} prerelease={set.prerelease} error={error} />
    </main>
  );
}
