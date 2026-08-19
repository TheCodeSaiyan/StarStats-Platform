/**
 * Prometheus scrape endpoint for the Next.js web app.
 *
 * Returns the default registry plus our custom counters/histograms
 * in Prometheus exposition format. Voyager's prometheus instance
 * scrapes this every 30s.
 *
 * Public route — no auth. Same access posture as the API's
 * `/metrics`: it lives on the internal docker network and is
 * proxied through Traefik to private callers only. If you bind
 * the web service to a public host without filtering, scrape data
 * leaks. Don't.
 */

import { registry } from '@/lib/metrics';

export const dynamic = 'force-dynamic'; // never statically render
export const runtime = 'nodejs';        // prom-client needs Node APIs

export async function GET(): Promise<Response> {
  const body = await registry.metrics();
  return new Response(body, {
    status: 200,
    headers: {
      'content-type': registry.contentType,
      'cache-control': 'no-store',
    },
  });
}
