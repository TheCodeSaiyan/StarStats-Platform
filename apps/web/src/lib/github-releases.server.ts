/**
 * Server-only network read of the GitHub Releases API for the tray track.
 * Kept separate from the isomorphic parsing helpers in `github-releases.ts`
 * so the client `DownloadsView` can share types/formatters without pulling
 * `server-only` into a client bundle.
 */

import 'server-only';
import {
  RELEASES_API,
  mapReleases,
  selectReleases,
  type RawRelease,
  type TrayReleaseSet,
} from './github-releases';

/**
 * Where the release feed is read from.
 *
 * Overridable ONLY here, in the server-only module. `RELEASES_API` itself
 * lives in the isomorphic file that the client component imports for its
 * types and formatters, and a non-`NEXT_PUBLIC_` env var read from there
 * would be inlined as undefined in the client bundle.
 *
 * The override exists because `/downloads` is no longer a leaf marketing page:
 * it absorbed `/devices`, so the e2e suite now lands on it during auth flows
 * and pairing captures. Without it every one of those runs makes a real,
 * rate-limited call to api.github.com — flaky, slow, and dependent on the
 * network being reachable from CI.
 */
const RELEASES_ENDPOINT = process.env.STARSTATS_RELEASES_API || RELEASES_API;

export async function fetchTrayReleases(): Promise<TrayReleaseSet> {
  // Env-gated cache: prod keeps a 5-min data cache; CI/e2e disables it so
  // mock fixtures don't leak across scenarios (see reference.ts precedent).
  //
  // Was 1800 (30 min). That is the window right after a release when the
  // page is most read and most wrong: tray 0.1.15 shipped and /downloads
  // went on offering 0.1.13 for a measured 30 minutes, flipping at exactly
  // 1800s. Five minutes is 12 requests/hour against GitHub's 60/hour
  // unauthenticated limit — still an order of magnitude of headroom, and
  // React's per-render dedup means one render never costs more than one
  // upstream call regardless.
  const cacheOpts = process.env.STARSTATS_DISABLE_FETCH_CACHE
    ? { cache: 'no-store' as const }
    : { next: { revalidate: 300 } };

  const res = await fetch(`${RELEASES_ENDPOINT}?per_page=30`, {
    headers: {
      Accept: 'application/vnd.github+json',
      'X-GitHub-Api-Version': '2022-11-28',
    },
    ...cacheOpts,
  });
  if (!res.ok) {
    throw new Error(`github releases fetch → ${res.status}`);
  }
  const raw = (await res.json()) as RawRelease[];
  return selectReleases(mapReleases(raw));
}
