/**
 * /metrics — deprecated. Per design audit v2 §07 the manifest viewer
 * was merged into `/journey?view=types`, which is itself now superseded
 * by /me (Mirror Plan 4). This stub redirects all incoming traffic
 * (external bookmarks, login `next=` targets, etc.) to the unified home.
 */
import { redirect } from 'next/navigation';

export const metadata = { title: "Metrics" };

export default function MetricsPage() {
  redirect('/me');
}
