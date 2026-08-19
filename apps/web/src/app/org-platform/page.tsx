import { redirect } from 'next/navigation';
import type { Route } from 'next';

/**
 * Legacy route. The "Org platform" product was renamed to StarPlatform;
 * this permanently redirects to `/star-platform` so existing links and the
 * old marketing URL keep working. The `as Route` cast covers the typed-
 * routes gap until the new route is registered by a dev/build cycle.
 */
export default function OrgPlatformRedirect() {
  redirect('/star-platform' as Route);
}
