import { redirect } from 'next/navigation';

export const metadata = { title: "Dashboard" };

// Superseded by /me (Mirror Plan 4). Kept as a redirect so existing
// links/bookmarks land on the unified home.
export default function DashboardRedirect() {
  redirect('/me');
}
