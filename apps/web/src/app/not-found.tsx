/**
 * 404 page. Server Component — renders the same branded chrome as the
 * rest of the app and offers a context-aware "go back" link: signed-in
 * users go to the dashboard, signed-out users go to the marketing home.
 */

import Link from 'next/link';
import { getSession } from '@/lib/session';
import { BoundaryShell } from '@/components/projection/BoundaryShell';

export default async function NotFound() {
  const session = await getSession();
  const target = session ? '/me' : '/';
  const targetLabel = session ? 'Back to overview' : 'Back to home';

  // The crumb carries the `<h1>`. Copy is unchanged — including the
  // context-aware target, which is the whole point of this page being a server
  // component rather than a static one.
  return (
    <BoundaryShell crumb="Page not found">
      <p className="hp-prose">
        We couldn&apos;t find the page you were looking for. It may have
        moved, or the link might be stale.
      </p>
      <p className="hp-prose">
        <Link href={target}>{targetLabel}</Link>
      </p>
    </BoundaryShell>
  );
}
