import 'server-only';
import type { Route } from 'next';
import { redirect } from 'next/navigation';
import { getCurrentHandle } from '@/lib/api';
import { logger } from '@/lib/logger';

/**
 * Send a link built on a handle its owner has renamed away from to the
 * owner's current handle. The server only answers for the 90 days the old
 * handle is held; after that, or for a live or unknown handle, this
 * returns and the caller renders whatever it would have.
 *
 * Call it only on a path that already failed to find the handle (a
 * denied or not-found render), so a normal profile view costs nothing.
 * `redirect()` throws, so it sits outside the try.
 */
export async function redirectIfRenamed(
  handle: string,
  pathFor: (current: string) => string,
): Promise<void> {
  let current: string | null = null;
  try {
    current = await getCurrentHandle(handle);
  } catch (e) {
    logger.warn({ err: e }, 'call=handle_current failed');
    return;
  }
  if (current && current.toLowerCase() !== handle.toLowerCase()) {
    redirect(pathFor(current) as Route);
  }
}
