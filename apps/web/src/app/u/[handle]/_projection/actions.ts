'use server';

import { getSession } from '@/lib/session';
import { getProfileLayout, updateProfileLayout, type LayoutEntry } from '@/lib/api';
import { logger } from '@/lib/logger';
import { PROFILE_ELEMENT_IDS } from './catalogue';

export type SaveLayoutResult = { ok: true } | { ok: false; error: string };

/**
 * Persist the arrangement of the owner's public profile.
 *
 * The mirror of `saveProjectionLayoutAction` for the `profile` surface, and it
 * keeps that action's two careful properties:
 *
 *   - Only ids THIS surface can draw are honoured. Anything else would be
 *     written straight back and could then never be turned off from here.
 *   - Entries the surface does not manage are carried forward untouched, and
 *     so is any geometry on the ones it does.
 *
 * The geometry is now dead weight rather than load-bearing: `/u/[handle]`
 * renders a flat stack in layout order and reads no `x/y/w/h`. It is preserved
 * anyway, because it is the only record of an arrangement an owner made in the
 * old free-grid editor, and discarding it as a side effect of a reorder would
 * be destroying something on their behalf that they never asked us to touch.
 */
export async function saveProfileLayoutAction(
  ids: string[],
): Promise<SaveLayoutResult> {
  const session = await getSession();
  if (!session) return { ok: false, error: 'not_authenticated' };

  const known = new Set<string>(PROFILE_ELEMENT_IDS);
  const enabled = ids.filter((id) => known.has(id));

  try {
    const current = await getProfileLayout(session.token, 'profile');
    const stored = current.layout ?? [];
    const byId = new Map(stored.map((e) => [e.id, e] as const));

    const next: LayoutEntry[] = [];
    // Enabled entries first, in the order the owner chose — that order IS the
    // order the dock renders in.
    for (const id of enabled) {
      const prev = byId.get(id);
      next.push({
        ...(prev ?? { id, size: 'compact' as const }),
        id,
        enabled: true,
      });
      byId.delete(id);
    }
    // Everything else carried forward, disabled only if this surface manages it.
    for (const [, entry] of byId) {
      next.push({
        ...entry,
        enabled: known.has(entry.id) ? false : entry.enabled,
      });
    }

    await updateProfileLayout(session.token, next, 'profile');
    return { ok: true };
  } catch (err) {
    logger.warn(
      { err, call: 'action.saveProfileLayout' },
      'save profile layout failed',
    );
    return { ok: false, error: 'save_failed' };
  }
}
