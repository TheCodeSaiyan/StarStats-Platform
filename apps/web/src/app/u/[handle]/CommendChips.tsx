import React from 'react';
import { ApiCallError, getProfileCommends } from '@/lib/api';
import { commendLabel } from '@/lib/commends';
import { logger } from '@/lib/logger';

/**
 * Crew commends on a profile: one chip per word this player has been given,
 * with its total. Only crewmates can give them, and nobody is shown who
 * gave which, the owner included. Renders nothing when there are none or
 * the totals cannot be read, so an outage costs the chips and not the page.
 */
export async function CommendChips({ handle, token }: { handle: string; token: string | null }) {
  let totals;
  try {
    totals = (await getProfileCommends(handle, token ?? undefined)).totals;
  } catch (e) {
    if (!(e instanceof ApiCallError) || e.status !== 404) {
      logger.warn({ err: e, call: 'profile.commends' }, 'commend totals fetch failed');
    }
    return null;
  }
  const given = totals.filter((t) => t.count > 0);
  if (given.length === 0) return null;
  return (
    <>
      {given.map((t) => (
        <span
          key={t.kind}
          className="hp-chip"
          data-testid="commend-total"
          title={`Commended "${commendLabel(t.kind)}" by crewmates ${t.count} ${t.count === 1 ? 'time' : 'times'}`}
        >
          {commendLabel(t.kind)} · {t.count}
        </span>
      ))}
    </>
  );
}
