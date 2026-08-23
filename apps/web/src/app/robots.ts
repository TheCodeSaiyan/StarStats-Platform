import type { MetadataRoute } from 'next';

import { isNoindexDeployment } from '@/lib/deployment';

/**
 * `/robots.txt`.
 *
 * Staging deployments (beta.starstats.app) set `STARSTATS_NOINDEX=1`
 * and serve a blanket Disallow so the unfinished UI never enters a
 * search index and never competes with starstats.app for the same
 * content. Production leaves the flag unset and gets the normal
 * allow-all.
 *
 * `force-dynamic` is load-bearing. Next statically generates
 * `robots.txt` at build time by default, which would bake whichever
 * value `STARSTATS_NOINDEX` happened to have in the CI runner into the
 * image — and the whole point is that ONE image serves both prod and
 * beta, differing only by container env. Same reasoning as
 * `STARSTATS_SITE_URL` in `app/layout.tsx`.
 */
export const dynamic = 'force-dynamic';

export default function robots(): MetadataRoute.Robots {
  const siteUrl = process.env.STARSTATS_SITE_URL ?? 'https://starstats.app';

  if (isNoindexDeployment()) {
    return {
      rules: [{ userAgent: '*', disallow: '/' }],
    };
  }

  return {
    rules: [
      {
        userAgent: '*',
        allow: '/',
        // Authenticated surfaces and machine endpoints carry nothing a
        // crawler should hold, and `/api/*` includes the healthcheck
        // the container probes.
        disallow: ['/api/', '/admin/', '/settings/', '/me/'],
      },
    ],
    host: siteUrl,
  };
}
