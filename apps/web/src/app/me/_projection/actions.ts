'use server';

import { getSession } from '@/lib/session';
import { updateProfileLayout, type LayoutEntry } from '@/lib/api';
import { isTheme, setTheme } from '@/lib/theme';
import { logger } from '@/lib/logger';
import { PROJECTION_IDS } from './catalogue';

/**
 * Persist the projection layout on the ACCOUNT.
 *
 * The design kit persisted to localStorage and flags that as a stand-in: the
 * product's own guide is explicit that a reader's layout "follows you to
 * another browser". Writing to localStorage here would have silently downgraded
 * a working cross-device behaviour to a per-device one, so this goes to the
 * same `PUT /v1/users/me/profile-layout?surface=home` the flat dashboard uses.
 *
 * The projection reasons in ordered ids; the API stores `LayoutEntry[]`. This
 * adapts between them and — importantly — PRESERVES the geometry fields on
 * entries that already have them. The projection does not read `x/y/w/h`, but
 * dropping them would destroy a reader's flat-dashboard arrangement as a side
 * effect of visiting the new page, and `/u/[handle]` still renders from it.
 */
export type SaveLayoutResult =
  | { ok: true }
  | { ok: false; error: string };

export async function saveProjectionLayoutAction(
  ids: string[],
): Promise<SaveLayoutResult> {
  const session = await getSession();
  if (!session) return { ok: false, error: 'not_authenticated' };

  // Only ids the projection actually knows how to draw; anything else would be
  // stored back verbatim and could not be turned off from this surface.
  const known = new Set<string>(PROJECTION_IDS);
  const enabled = ids.filter((id) => known.has(id));

  try {
    // Read the stored layout first so entries the projection does not manage
    // (and any geometry on the ones it does) survive the write.
    const current = await import('@/lib/api').then((m) =>
      m.getProfileLayout(session.token, 'home'),
    );
    const stored = current.layout ?? [];
    const byId = new Map(stored.map((e) => [e.id, e] as const));

    const next: LayoutEntry[] = [];
    // Enabled entries first, in the reader's chosen order.
    for (const id of enabled) {
      const prev = byId.get(id);
      next.push({ ...(prev ?? { id, size: 'compact' as const }), id, enabled: true });
      byId.delete(id);
    }
    // Everything else is carried forward disabled, keeping its geometry.
    for (const [, entry] of byId) {
      next.push({ ...entry, enabled: known.has(entry.id) ? false : entry.enabled });
    }

    await updateProfileLayout(session.token, next, 'home');
    return { ok: true };
  } catch (err) {
    logger.warn(
      { err, call: 'action.saveProjectionLayout' },
      'save projection layout failed',
    );
    return { ok: false, error: 'save_failed' };
  }
}

/**
 * Persist a calibration.
 *
 * Calibrations map one-to-one onto the existing theme model — `terra`,
 * `stanton`, `pyro`, `nyx` are already the four `Theme` values, already
 * persisted to the `ss-theme` cookie and `/v1/me/preferences`. So `data-cal`
 * reuses that machinery wholesale rather than introducing a parallel store;
 * only the ATTRIBUTE name changes, and `data-theme` is never reintroduced.
 */
export async function setCalibrationAction(
  calibration: string,
): Promise<{ ok: boolean }> {
  if (!isTheme(calibration)) return { ok: false };
  const session = await getSession();
  try {
    await setTheme(calibration, session?.token);
    return { ok: true };
  } catch (err) {
    logger.warn({ err, call: 'action.setCalibration' }, 'set calibration failed');
    return { ok: false };
  }
}
