'use server';

import { getSession } from '@/lib/session';
import {
  updateProfileLayout,
  type LayoutEntry,
  type LayoutSurface,
  type ProfileLayoutResponse,
} from '@/lib/api';
import { logger } from '@/lib/logger';

export type SaveLayoutResult =
  | { ok: true; layout: LayoutEntry[] | null }
  | { ok: false; error: string };

/**
 * Persist a new layout for the current owner on the given surface
 * ('profile' = public /u/[handle], 'home' = private /me). Returns the
 * server's canonical form (it may sanitise unknown ids).
 *
 * PERF: intentionally does NOT `revalidatePath`. The client updates its
 * layout optimistically (SortableProfileWidgets `save` → `setLayout`
 * before awaiting this action), so revalidation would only re-run every
 * widget's server-side data fetch — the 10–30s "saving layout" stall.
 * The persisted layout is picked up on the next full page load.
 */
export async function saveProfileLayoutAction(
  layout: LayoutEntry[] | null,
  surface: LayoutSurface = 'profile',
): Promise<SaveLayoutResult> {
  const session = await getSession();
  if (!session) {
    return { ok: false, error: 'not_authenticated' };
  }
  try {
    const res: ProfileLayoutResponse = await updateProfileLayout(
      session.token,
      layout,
      surface,
    );
    return { ok: true, layout: res.layout ?? null };
  } catch (err) {
    logger.warn(
      { err, call: 'action.saveProfileLayout', surface },
      'save layout failed',
    );
    return { ok: false, error: 'save_failed' };
  }
}
