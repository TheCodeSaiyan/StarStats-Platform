/**
 * `GET /settings/export?format=ndjson|csv|zip` — the manifest download.
 *
 * The browser cannot call the API itself: `apiBase()` is an internal
 * compose hostname, and the bearer token lives in the HttpOnly session
 * cookie. So this handler is the one place in the web tier that carries
 * the session token upstream and streams bytes back — `upstream.body` is
 * passed through untouched, so a large export never sits in this
 * process's memory.
 *
 * Failures redirect back to the Retention pane with an `?error=` code the
 * page already knows how to render, rather than landing the user on a
 * bare error body where a file was expected. Same relative-`Location`
 * 302 as `auth/logout`: `req.url` is the container's own origin here, not
 * the public one, so an absolute redirect would point at the wrong host.
 */
import type { NextRequest } from 'next/server';
import { getSession } from '@/lib/session';
import { exportManifest, isExportFormat } from '@/lib/api';

export const dynamic = 'force-dynamic';

function found(location: string): Response {
  return new Response(null, { status: 302, headers: { location } });
}

export async function GET(req: NextRequest): Promise<Response> {
  const format = req.nextUrl.searchParams.get('format');
  if (!isExportFormat(format)) {
    return new Response('unknown export format', { status: 400 });
  }

  const session = await getSession();
  if (!session) return found('/auth/login?next=/settings');

  let upstream: Response;
  try {
    upstream = await exportManifest(session.token, format);
  } catch {
    return found('/settings?error=export_failed#retention');
  }

  if (upstream.status === 401) return found('/auth/login?next=/settings');
  if (upstream.status === 429) {
    return found('/settings?error=export_too_soon#retention');
  }
  if (!upstream.ok || !upstream.body) {
    return found('/settings?error=export_failed#retention');
  }

  const headers: Record<string, string> = {
    'content-type':
      upstream.headers.get('content-type') ?? 'application/octet-stream',
    'cache-control': 'no-store',
    'x-content-type-options': 'nosniff',
  };
  const disposition = upstream.headers.get('content-disposition');
  if (disposition) headers['content-disposition'] = disposition;

  return new Response(upstream.body, { status: 200, headers });
}
