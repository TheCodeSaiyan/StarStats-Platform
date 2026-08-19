/**
 * Same-origin proxy for cohort member vectors (bulk-add). Mirrors the
 * /kb/compare proxy — the browser can't hit `apiBase()` (server-only
 * internal hostname), so we proxy to the Rust /cohort endpoint
 * server-side. Forwards upstream status + JSON.
 */

import { apiBase } from '@/lib/api';

const VALID = new Set(['vehicle', 'weapon', 'item', 'location']);

export async function GET(
  req: Request,
  { params }: { params: Promise<{ category: string }> },
): Promise<Response> {
  const { category } = await params;
  if (!VALID.has(category)) return new Response(null, { status: 404 });

  const key = new URL(req.url).searchParams.get('key') ?? '';
  if (!key) return new Response(JSON.stringify({ entries: [] }), { status: 200, headers: { 'content-type': 'application/json' } });

  const upstream = `${apiBase()}/v1/reference/${category}/cohort?key=${encodeURIComponent(key)}`;
  let resp: Response;
  try {
    resp = await fetch(upstream, { cache: 'no-store' });
  } catch {
    return new Response(JSON.stringify({ entries: [] }), { status: 200, headers: { 'content-type': 'application/json' } });
  }
  const body = await resp.text();
  return new Response(body, { status: resp.status, headers: { 'content-type': resp.headers.get('content-type') ?? 'application/json' } });
}
