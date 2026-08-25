import 'server-only';
import { headers } from 'next/headers';

/**
 * Headers that tell the API which end user a server-side render is for.
 *
 * WHY THIS EXISTS. The web tier is server-rendered, so every reference read
 * for every visitor reaches the API from the web container's single address.
 * The API's per-IP limiter therefore fronts the whole site with ONE bucket: a
 * crawler walking KB slugs does not throttle itself, it drains the bucket that
 * every real reader's page render shares, and those renders 429. The limit had
 * already been raised once for this and raising it again only moves the cliff.
 *
 * Exempting the renderer would be worse, not better — the crawler reaches the
 * API THROUGH this tier, so its traffic would arrive pre-approved. Instead the
 * renderer names the reader it is rendering for and that reader gets their own
 * bucket. The crawler is then throttled on its own address; everyone else is
 * unaffected by it.
 *
 * The API believes the forwarded address ONLY when the shared secret is
 * present (see `SsrAwareIpKeyExtractor` in `reference_routes.rs`), so this is
 * a claim the server verifies rather than one it takes on trust.
 *
 * INERT UNTIL BOTH SIDES ARE CONFIGURED. With no `STARSTATS_SSR_TOKEN` here
 * the headers are simply not sent; with none on the server they are ignored.
 * Either way the behaviour is exactly what it was.
 */
const TOKEN_HEADER = 'x-starstats-ssr';
const FOR_HEADER = 'x-starstats-ssr-for';

/**
 * The reader's address, from the proxy chain.
 *
 * `x-forwarded-for` is a comma-separated list appended to by each hop, and the
 * LEFTMOST entry is the original client. Taking the last would key every
 * reader by the same edge proxy and reinstate the shared bucket this exists to
 * remove.
 */
function clientAddress(h: Headers): string | null {
  const fwd = h.get('x-forwarded-for');
  if (fwd) {
    const first = fwd.split(',')[0]?.trim();
    if (first) return first;
  }
  return h.get('cf-connecting-ip') ?? h.get('x-real-ip');
}

export async function ssrIdentityHeaders(): Promise<Record<string, string>> {
  const token = process.env.STARSTATS_SSR_TOKEN;
  if (!token) return {};
  try {
    // `headers()` throws outside a request scope — during static generation,
    // or from a build-time call. There is no reader to name in that case, and
    // it must not take the render down.
    const h = await headers();
    const addr = clientAddress(h);
    return addr
      ? { [TOKEN_HEADER]: token, [FOR_HEADER]: addr }
      : { [TOKEN_HEADER]: token };
  } catch {
    return { [TOKEN_HEADER]: token };
  }
}
