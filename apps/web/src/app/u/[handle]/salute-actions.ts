'use server';

import type { Route } from 'next';
import { redirect } from 'next/navigation';
import { ApiCallError, saluteProfile, unsaluteProfile } from '@/lib/api';
import { logger } from '@/lib/logger';
import { getSession } from '@/lib/session';

/** Errors the page can explain; anything else reads as "try again". */
const KNOWN = new Set([
  'rsi_handle_not_verified',
  'rate_limited',
  'not_found',
  'spicedb_unavailable',
]);

/**
 * Salute or unsalute the profile named in the form. The page re-renders from
 * the server afterwards, so the count and the button show what was stored,
 * not what was clicked.
 */
export async function saluteAction(formData: FormData) {
  const handle = String(formData.get('handle') ?? '').trim();
  const back = `/u/${encodeURIComponent(handle)}`;
  const session = await getSession();
  if (!session) redirect(`/auth/login?next=${encodeURIComponent(back)}`);
  const intent = formData.get('intent') === 'unsalute' ? 'unsalute' : 'salute';
  try {
    if (intent === 'salute') await saluteProfile(session.token, handle);
    else await unsaluteProfile(session.token, handle);
  } catch (e) {
    if (e instanceof ApiCallError) {
      if (e.status === 401) redirect(`/auth/login?next=${encodeURIComponent(back)}`);
      const code = KNOWN.has(e.body.error) ? e.body.error : 'unexpected';
      logger.warn({ err: e, call: `profile.${intent}`, status: e.status }, 'salute failed');
      redirect(`${back}?salute_error=${code}` as Route);
    }
    logger.error({ err: e, call: `profile.${intent}` }, 'salute failed');
    redirect(`${back}?salute_error=unexpected` as Route);
  }
  redirect(back as Route);
}
