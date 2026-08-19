import 'server-only';
import { getWaitlistStatus } from '@/lib/api';
import { logger } from '@/lib/logger';

/**
 * Is the invite-only beta gate on?
 *
 * One helper so every surface that reacts to the gate reads it the same
 * way. The root layout, the login banner and the signup banner each
 * need this answer, and three copies of the fetch would drift — one of
 * them eventually failing open while the others fail closed.
 *
 * **Fails CLOSED (returns false) when the status cannot be read.** That
 * is deliberate and it is the opposite of the server's posture, for a
 * reason: the server refuses signup on an unreadable gate because
 * admitting the world is the dangerous direction there. Here the risk
 * runs the other way — a status blip must never trap visitors behind a
 * banner claiming the site is closed when it is open. The server is the
 * authoritative gate either way; this only decides what we SAY.
 */
export async function isBetaGateOn(): Promise<boolean> {
  try {
    const status = await getWaitlistStatus();
    return status.gate_enabled === true;
  } catch (err) {
    logger.warn({ err, call: 'beta-gate.status' }, 'waitlist status read failed');
    return false;
  }
}
