/**
 * Same-origin proxy for the multi-ship comparison vectors.
 *
 * The browser can't hit `apiBase()` (server-only internal hostname), so
 * the client fetches this relative route and we proxy to the Rust
 * `/v1/reference/{category}/compare?slugs=…` server-side. Forwards the
 * upstream status (notably 400 on bad/over-cap slug lists) and JSON body.
 */

import { apiBase } from '@/lib/api';

const VALID = new Set(['vehicle', 'weapon', 'item', 'location']);

export async function GET(
  req: Request,
  { params }: { params: Promise<{ category: string }> },
): Promise<Response> {
  const { category } = await params;
  if (!VALID.has(category)) return new Response(null, { status: 404 });

  const slugs = new URL(req.url).searchParams.get('slugs') ?? '';
  if (!slugs) {
    return new Response(JSON.stringify({ entries: [] }), {
      status: 200,
      headers: { 'content-type': 'application/json' },
    });
  }

  const upstream = `${apiBase()}/v1/reference/${category}/compare?slugs=${encodeURIComponent(slugs)}`;
  let resp: Response;
  try {
    resp = await fetch(upstream, { cache: 'no-store' });
  } catch {
    return new Response(JSON.stringify({ entries: [] }), {
      status: 200,
      headers: { 'content-type': 'application/json' },
    });
  }
  const body = await resp.text();
  return new Response(body, {
    status: resp.status,
    headers: { 'content-type': resp.headers.get('content-type') ?? 'application/json' },
  });
}
