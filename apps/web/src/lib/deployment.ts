/**
 * Deployment-identity helpers.
 *
 * One `web` image serves every environment; what differs is the
 * container env. These read that env at RUNTIME (never a
 * `NEXT_PUBLIC_*` prefix, which webpack would inline at build time and
 * freeze into the image — see the metadataBase note in
 * `app/layout.tsx`).
 */

/**
 * True on a staging deployment that must stay out of search indexes —
 * today `beta.starstats.app`, set via `STARSTATS_NOINDEX=1`.
 *
 * Drives both `/robots.txt` (`app/robots.ts`) and the per-page
 * `<meta name="robots">` tag (`app/layout.tsx`). Both are needed:
 * robots.txt stops well-behaved crawlers fetching, the meta tag stops
 * indexing of URLs discovered by other means (inbound links, sitemaps
 * held elsewhere, Chrome telemetry).
 */
export function isNoindexDeployment(): boolean {
  return process.env.STARSTATS_NOINDEX === '1';
}
