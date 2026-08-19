/**
 * Browser-side proxy for `GET /v1/discover/profiles`.
 *
 * The Rust API URL lives in `STARSTATS_API_URL`, a server-only env
 * var — the browser can't reach it directly. The /discover "Load
 * more" client component therefore fetches this Next route handler,
 * which runs in the Next Node runtime and forwards to the Rust
 * endpoint using the existing server-side fetcher. The data shape
 * is identical to the upstream response so the client never has to
 * know there's a proxy.
 *
 * The Rust endpoint is unauthenticated by design (Piece 3) so this
 * proxy carries no credentials — no token rebroadcasting, no
 * cookie passthrough.
 */

import { NextResponse, type NextRequest } from 'next/server';
import { ApiCallError, getDiscoverProfiles } from '@/lib/api';
import { logger } from '@/lib/logger';

export const dynamic = 'force-dynamic';

export async function GET(req: NextRequest) {
  const url = new URL(req.url);
  const after = url.searchParams.get('after') ?? undefined;
  const limitRaw = url.searchParams.get('limit');
  const limit = limitRaw === null ? undefined : Number(limitRaw);

  try {
    const body = await getDiscoverProfiles({
      after,
      // Drop NaN; the upstream clamps anyway, but we don't want to
      // send a literal `?limit=NaN` if the client passed garbage.
      limit: limit !== undefined && Number.isFinite(limit) ? limit : undefined,
    });
    return NextResponse.json(body, { status: 200 });
  } catch (e) {
    const status = e instanceof ApiCallError ? e.status : 502;
    logger.error(
      { err: e, call: 'discoverProxy', status },
      'discover proxy fetch failed',
    );
    return NextResponse.json(
      { error: e instanceof ApiCallError ? e.body.error : 'upstream_error' },
      { status },
    );
  }
}
