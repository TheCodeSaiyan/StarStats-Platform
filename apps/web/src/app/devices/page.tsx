/**
 * `/devices` has been folded into `/downloads` — the Emitter.
 *
 * The design system treats the desktop client as one thing across its whole
 * lifecycle: download it, pair it, watch what it sends, revoke it. Splitting
 * "get the client" from "make the client do something" across two destinations
 * meant a reader who had just downloaded the tray had to go looking for a
 * second page to use it. The pairing flow, the per-device tabs, the two-gate
 * cloud-sync toggle and the ingest activity table all moved verbatim.
 *
 * This redirect is not decoration. `/devices` is referenced by the terms of
 * service, two guides, the features page, the signup flow's post-verification
 * hop and the fleet pane's refresh affordance. It stays a working URL — but,
 * per the system's rule that a permanent redirect is never offered as a
 * destination, it is no longer in the nav model.
 *
 * The query string carries over: `?device=<id>` pins a device tab and
 * `?code=`/`?expires=` carry a freshly minted pairing code, and dropping them
 * would silently strand anyone who arrived on a deep link.
 */
import { redirect } from 'next/navigation';

export default async function DevicesRedirect(props: {
  searchParams: Promise<Record<string, string | string[] | undefined>>;
}) {
  const params = await props.searchParams;
  const qs = new URLSearchParams();
  for (const [k, v] of Object.entries(params)) {
    if (typeof v === 'string') qs.set(k, v);
    else if (Array.isArray(v) && v[0] != null) qs.set(k, v[0]);
  }
  const query = qs.toString();
  redirect(query ? `/downloads?${query}` : '/downloads');
}
