'use server';

import { joinWaitlist } from '@/lib/api';
import { logger } from '@/lib/logger';

export type JoinWaitlistResult =
  | { ok: true; position: number | null }
  | { ok: false; error: string };

/**
 * Mirrors the server's `looks_like_email` in waitlist_routes.rs. A cheap
 * client-side reject so an obvious typo doesn't cost a round trip — the
 * server still validates, and its answer is the one that counts.
 */
function looksLikeEmail(s: string): boolean {
  const t = s.trim();
  if (t.length < 3 || t.length > 254) return false;
  const parts = t.split('@');
  if (parts.length !== 2) return false;
  const [local, domain] = parts;
  return (
    local.length > 0 &&
    domain.includes('.') &&
    !domain.startsWith('.') &&
    !domain.endsWith('.')
  );
}

/**
 * Join the public-beta waitlist.
 *
 * `position: null` means admitted immediately — the invite is already in
 * the post. A number means queued at that 1-based position.
 */
export async function joinWaitlistAction(
  formData: FormData,
): Promise<JoinWaitlistResult> {
  const email = String(formData.get('email') ?? '').trim();
  const source = String(formData.get('source') ?? '').trim() || undefined;

  if (!looksLikeEmail(email)) {
    return { ok: false, error: 'invalid_email' };
  }

  try {
    const resp = await joinWaitlist({ email, source });
    // `?? null`, not `?? 0`: the server omits `position` on admit, and a
    // numeric fallback would tell an admitted user they are "number 0 in
    // the queue".
    return { ok: true, position: resp.position ?? null };
  } catch (err) {
    logger.warn({ err, call: 'action.joinWaitlist' }, 'waitlist join failed');
    return { ok: false, error: 'join_failed' };
  }
}
