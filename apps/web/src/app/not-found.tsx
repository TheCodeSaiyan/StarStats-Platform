/**
 * 404 page. Server Component — renders the same branded chrome as the
 * rest of the app and offers a context-aware "go back" link: signed-in
 * users go to the dashboard, signed-out users go to the marketing home.
 */

import Link from 'next/link';
import { getSession } from '@/lib/session';

export default async function NotFound() {
  const session = await getSession();
  const target = session ? '/me' : '/';
  const targetLabel = session ? 'Back to overview' : 'Back to home';

  return (
    <main>
      <h1>Page not found</h1>
      <p className="muted">
        We couldn&apos;t find the page you were looking for. It may have
        moved, or the link might be stale.
      </p>
      <p>
        <Link href={target}>{targetLabel}</Link>
      </p>
    </main>
  );
}
