'use server';

/**
 * Server actions used by the /sharing page. Currently just the
 * "Report a share" action (audit v2 §05 — recipient-facing report
 * affordance on each inbound-share row).
 *
 * The reporter handle is taken off the session bearer token by the
 * server — the form never sends a reporter field, so spoofing one
 * user as another is impossible from the client.
 *
 * The action redirects back to /sharing with a `?status=report_filed`
 * or `?error=...` param so the outcome surfaces as a chip — success and
 * failure both give explicit feedback.
 */

import { revalidatePath } from 'next/cache';
import { redirect } from 'next/navigation';
import {
  addShare,
  ApiCallError,
  listShares,
  removeShare,
  reportShare,
  type ShareScope,
} from '@/lib/api';
import { logger } from '@/lib/logger';
import { getSession } from '@/lib/session';

const LOGIN_NEXT = '/auth/login?next=/sharing';

export async function reportShareAction(formData: FormData): Promise<void> {
  const session = await getSession();
  if (!session) redirect(LOGIN_NEXT);

  const owner_handle = String(formData.get('owner_handle') ?? '').trim();
  // The reporter IS the auth'd recipient — take their handle off the
  // session, never a client-supplied hidden input (server-side handle
  // truth). The client may only name the OTHER party (owner_handle). (M-W2)
  const recipient_handle = session.claimedHandle.trim();
  const reason = String(formData.get('reason') ?? '').trim();
  const detailsRaw = formData.get('details');
  const details =
    typeof detailsRaw === 'string' && detailsRaw.trim().length > 0
      ? detailsRaw.trim()
      : undefined;

  if (!owner_handle || !recipient_handle || !reason) {
    redirect('/sharing?error=report_invalid');
  }

  try {
    await reportShare(session.token, {
      owner_handle,
      recipient_handle,
      reason,
      details,
    });
  } catch (e) {
    if (e instanceof ApiCallError) {
      if (e.status === 401) redirect(LOGIN_NEXT);
      if (e.status === 403 || e.status === 400 || e.status === 429) {
        redirect('/sharing?error=report_failed');
      }
    }
    throw e;
  }
  // Give explicit success feedback — the action previously returned
  // silently on both success AND failure. (M-W2)
  redirect('/sharing?status=report_filed');
}

// -- Audit v2.1 §A3 — bulk operations on outbound shares --------------

const BULK_SCOPE_KINDS = new Set(['full', 'timeline', 'aggregates', 'tabs']);

/**
 * Revoke every outbound user-to-user share whose `expires_at` is in
 * the past. Fans out individual `removeShare` calls so each gets its
 * own audit-log row server-side — the moderator surface keeps a
 * complete trail.
 *
 * Failures are tolerated: a single revoke that 404s or 503s logs but
 * doesn't poison the whole batch. The redirect carries the count of
 * shares that were actually revoked so the user sees progress.
 */
export async function bulkRevokeExpiredAction(): Promise<void> {
  const session = await getSession();
  if (!session) redirect(LOGIN_NEXT);

  let inventory;
  try {
    inventory = await listShares(session.token);
  } catch (e) {
    if (e instanceof ApiCallError && e.status === 401) redirect(LOGIN_NEXT);
    logger.error({ err: e }, 'bulkRevokeExpired list failed');
    redirect('/sharing?status=bulk_revoke_failed');
  }

  const now = Date.now();
  const expired = (inventory?.shares ?? []).filter(
    (s) => s.expires_at && new Date(s.expires_at).getTime() <= now,
  );

  let revoked = 0;
  for (const share of expired) {
    try {
      await removeShare(session.token, share.recipient_handle);
      revoked++;
    } catch (e) {
      logger.warn(
        { err: e, recipient: share.recipient_handle },
        'bulkRevokeExpired single revoke failed; continuing',
      );
    }
  }

  revalidatePath('/sharing');
  redirect(`/sharing?status=bulk_revoked&n=${revoked}`);
}

/**
 * Reset the scope on every active outbound user-to-user share to a
 * single chosen kind (default `aggregates`). Re-uses `addShare`
 * (upsert semantics) so each call lands as a normal scope-change
 * audit row — same provenance shape as a hand-edited grant. Note
 * and expiry are preserved on each row; only the scope changes.
 *
 * Expired shares are skipped — the user almost certainly wants
 * `bulkRevokeExpiredAction` for those, not a scope reset on a
 * row that's about to be revoked anyway.
 */
export async function bulkResetScopeAction(formData: FormData): Promise<void> {
  const session = await getSession();
  if (!session) redirect(LOGIN_NEXT);

  const kindRaw = String(formData.get('scope_kind') ?? 'aggregates').trim();
  if (!BULK_SCOPE_KINDS.has(kindRaw)) {
    redirect('/sharing?status=bulk_scope_reset_failed');
  }
  const kind = kindRaw as ShareScope['kind'];

  let inventory;
  try {
    inventory = await listShares(session.token);
  } catch (e) {
    if (e instanceof ApiCallError && e.status === 401) redirect(LOGIN_NEXT);
    logger.error({ err: e }, 'bulkResetScope list failed');
    redirect('/sharing?status=bulk_scope_reset_failed');
  }

  const now = Date.now();
  const active = (inventory?.shares ?? []).filter(
    (s) => !s.expires_at || new Date(s.expires_at).getTime() > now,
  );

  let reset = 0;
  for (const share of active) {
    try {
      await addShare(session.token, share.recipient_handle, {
        expiresAt: share.expires_at ?? null,
        note: share.note ?? null,
        scope: { kind },
      });
      reset++;
    } catch (e) {
      logger.warn(
        { err: e, recipient: share.recipient_handle },
        'bulkResetScope single update failed; continuing',
      );
    }
  }

  revalidatePath('/sharing');
  redirect(`/sharing?status=bulk_scope_reset&n=${reset}`);
}
