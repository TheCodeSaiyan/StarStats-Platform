'use server';

import { redirect } from 'next/navigation';
import {
  ApiCallError,
  adminCreateNews,
  adminDeleteNews,
  adminSetNewsPublished,
  adminUpdateNews,
} from '@/lib/api';
import { logger } from '@/lib/logger';
import { getSession } from '@/lib/session';

const LOGIN_NEXT = '/auth/login?next=/admin/news';

/** Server validation codes the page has copy for. */
const KNOWN = new Set([
  'title_required',
  'title_too_long',
  'body_required',
  'body_too_long',
  'invalid_link',
  'news_not_found',
]);

async function token(): Promise<string> {
  const s = await getSession();
  if (!s) redirect(LOGIN_NEXT);
  return s.token;
}

function fail(e: unknown, call: string): never {
  if (e instanceof ApiCallError) {
    if (e.status === 401) redirect(LOGIN_NEXT);
    if (KNOWN.has(e.body.error)) redirect(`/admin/news?error=${e.body.error}`);
    logger.error({ err: e, call, status: e.status }, 'news admin action failed');
  } else {
    logger.error({ err: e, call }, 'news admin action failed');
  }
  redirect('/admin/news?error=unexpected');
}

function fields(formData: FormData) {
  const link = String(formData.get('link_url') ?? '').trim();
  return {
    title: String(formData.get('title') ?? ''),
    body: String(formData.get('body') ?? ''),
    link_url: link === '' ? null : link,
  };
}

export async function createNewsAction(formData: FormData) {
  const t = await token();
  const publish = formData.get('publish') === 'on';
  let published: boolean;
  try {
    published = (await adminCreateNews(t, { ...fields(formData), publish })).published_at != null;
  } catch (e) {
    fail(e, 'news.create');
  }
  // From the response: a post that came back unpublished says so.
  redirect(`/admin/news?status=${published ? 'published' : 'drafted'}`);
}

export async function updateNewsAction(formData: FormData) {
  const t = await token();
  try {
    await adminUpdateNews(t, String(formData.get('id') ?? ''), fields(formData));
  } catch (e) {
    fail(e, 'news.update');
  }
  redirect('/admin/news?status=saved');
}

export async function setPublishedAction(formData: FormData) {
  const t = await token();
  const want = formData.get('published') === 'true';
  let live: boolean;
  try {
    live =
      (await adminSetNewsPublished(t, String(formData.get('id') ?? ''), want)).published_at !=
      null;
  } catch (e) {
    fail(e, 'news.publish');
  }
  redirect(`/admin/news?status=${live ? 'published' : 'unpublished'}`);
}

export async function deleteNewsAction(formData: FormData) {
  const t = await token();
  try {
    await adminDeleteNews(t, String(formData.get('id') ?? ''));
  } catch (e) {
    fail(e, 'news.delete');
  }
  redirect('/admin/news?status=deleted');
}
