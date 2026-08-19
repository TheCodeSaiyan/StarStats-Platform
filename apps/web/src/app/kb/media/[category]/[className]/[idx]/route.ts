/**
 * Same-origin image proxy for reference entity media.
 *
 * The browser must NOT hit the API origin directly for entity images:
 * `apiBase()` is the SERVER-SIDE base (`STARSTATS_API_URL`, e.g. the
 * internal `http://starstats-api:8080` compose hostname), which a client
 * can't resolve — putting it in an `<img src>` yields a DNS failure and a
 * broken image. So the gallery points at this RELATIVE route on the web
 * origin, and we proxy to the API server-side (where the internal
 * hostname resolves), streaming the bytes back.
 *
 * Upstream routes per category:
 *  - vehicle: `/v1/reference/vehicles/{class_name}/media/{idx}` (legacy shape)
 *  - item, weapon: `/v1/reference/{category}/{class_name}/media/{idx}` (Task 2 route)
 *
 * We forward the upstream status verbatim (notably 404 when media is
 * disabled or the entity has no media) so callers degrade gracefully.
 */

import { apiBase } from '@/lib/api';

// Categories that expose a media proxy endpoint.
const MEDIA_CATEGORIES = new Set(['vehicle', 'item', 'weapon']);

export async function GET(
  _req: Request,
  { params }: { params: Promise<{ category: string; className: string; idx: string }> },
): Promise<Response> {
  const { category, className, idx } = await params;

  if (!MEDIA_CATEGORIES.has(category)) {
    return new Response(null, { status: 404 });
  }
  // idx must be a non-negative integer — bounce obvious garbage.
  if (!/^\d+$/.test(idx)) {
    return new Response(null, { status: 404 });
  }

  // Vehicles use the legacy upstream path; items and weapons use the new
  // generic reference media route added in Task 2.
  const encodedClass = encodeURIComponent(className);
  const upstream =
    category === 'vehicle'
      ? `${apiBase()}/v1/reference/vehicles/${encodedClass}/media/${idx}`
      : `${apiBase()}/v1/reference/${category}/${encodedClass}/media/${idx}`;

  let resp: Response;
  try {
    resp = await fetch(upstream, { cache: 'no-store' });
  } catch {
    return new Response(null, { status: 502 });
  }

  if (!resp.ok) {
    // Forward the upstream status (404 = kill-switch off / no such image)
    // so the gallery hides the broken tile rather than retrying.
    return new Response(null, { status: resp.status });
  }

  const body = await resp.arrayBuffer();
  return new Response(body, {
    status: 200,
    headers: {
      'content-type': resp.headers.get('content-type') ?? 'image/jpeg',
      // Mirror the upstream's day-long cache so the browser/CDN don't
      // re-proxy every render.
      'cache-control': resp.headers.get('cache-control') ?? 'public, max-age=86400',
    },
  });
}
