'use server';

import { redirect } from 'next/navigation';
import { ApiCallError, markNewsSeen, markReleaseSeen, markWhatsNewSeen } from '@/lib/api';
import { logger } from '@/lib/logger';
import { getSession } from '@/lib/session';

const LOGIN_NEXT = '/auth/login?next=/whats-new';

async function token(): Promise<string> {
  const s = await getSession();
  if (!s) redirect(LOGIN_NEXT);
  return s.token;
}

function fail(e: unknown, call: string): never {
  if (e instanceof ApiCallError && e.status === 401) redirect(LOGIN_NEXT);
  logger.error({ err: e, call }, 'whats-new action failed');
  redirect('/whats-new?error=unexpected');
}

export async function markNewsReadAction(formData: FormData) {
  const t = await token();
  try {
    await markNewsSeen(t, String(formData.get('id') ?? ''));
  } catch (e) {
    fail(e, 'whatsnew.news_seen');
  }
  redirect('/whats-new');
}

export async function markReleaseReadAction(formData: FormData) {
  const t = await token();
  try {
    await markReleaseSeen(t, String(formData.get('id') ?? ''));
  } catch (e) {
    fail(e, 'whatsnew.release_seen');
  }
  redirect('/whats-new');
}

export async function markItemReadAction(formData: FormData) {
  const t = await token();
  try {
    await markWhatsNewSeen(
      t,
      String(formData.get('roadmap_item_id') ?? ''),
      String(formData.get('changelog_entry_id') ?? ''),
    );
  } catch (e) {
    fail(e, 'whatsnew.item_seen');
  }
  redirect('/whats-new');
}

/** Mark every listed post and item read. Ids come from the page's own
 *  render of the reader's feed, so there is nothing else to trust. */
export async function markAllReadAction(formData: FormData) {
  const t = await token();
  const news = formData.getAll('news_id').map(String);
  const items = formData.getAll('item').map(String);
  const releases = formData.getAll('release_id').map(String);
  try {
    await Promise.all([
      ...news.map((id) => markNewsSeen(t, id)),
      ...releases.map((id) => markReleaseSeen(t, id)),
      ...items.map((pair) => {
        const [itemId, entryId] = pair.split(':');
        return markWhatsNewSeen(t, itemId, entryId);
      }),
    ]);
  } catch (e) {
    fail(e, 'whatsnew.all_seen');
  }
  redirect('/whats-new?status=all_read');
}
