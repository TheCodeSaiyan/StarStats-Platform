import { describe, it, expect, vi, beforeEach } from 'vitest';

const joinWaitlist = vi.fn();
vi.mock('@/lib/api', () => ({
  joinWaitlist: (...a: unknown[]) => joinWaitlist(...a),
}));
vi.mock('@/lib/logger', () => ({
  logger: { warn: vi.fn(), info: vi.fn(), error: vi.fn() },
}));

import { joinWaitlistAction } from './waitlist';

const fd = (email: string, source?: string) => {
  const f = new FormData();
  f.set('email', email);
  if (source !== undefined) f.set('source', source);
  return f;
};

// Braces matter. `mockReset()` RETURNS the mock, and vitest treats a
// function returned from beforeEach as a teardown callback — so the
// concise-arrow form makes vitest call the mock itself after every test.
// With a rejecting implementation installed that produces a rejected
// promise nobody awaits, and the resulting unhandled rejection fails a
// test whose assertions all passed.
beforeEach(() => {
  joinWaitlist.mockReset();
});

describe('joinWaitlistAction', () => {
  it('returns the queue position when queued', async () => {
    joinWaitlist.mockResolvedValue({ joined: true, position: 7 });
    await expect(joinWaitlistAction(fd('a@example.com'))).resolves.toEqual({
      ok: true,
      position: 7,
    });
  });

  it('returns a null position when admitted immediately', async () => {
    joinWaitlist.mockResolvedValue({ joined: true, position: null });
    await expect(joinWaitlistAction(fd('a@example.com'))).resolves.toEqual({
      ok: true,
      position: null,
    });
  });

  it('treats a missing position as admitted rather than guessing a number', async () => {
    // The server omits `position` entirely on admit. Coercing that to 0
    // would render "number 0 in the queue" at someone who is actually in.
    joinWaitlist.mockResolvedValue({ joined: true });
    await expect(joinWaitlistAction(fd('a@example.com'))).resolves.toEqual({
      ok: true,
      position: null,
    });
  });

  it('rejects a malformed email without calling the api', async () => {
    await expect(joinWaitlistAction(fd('nope'))).resolves.toEqual({
      ok: false,
      error: 'invalid_email',
    });
    expect(joinWaitlist).not.toHaveBeenCalled();
  });

  it('rejects an empty email without calling the api', async () => {
    await expect(joinWaitlistAction(fd(''))).resolves.toEqual({
      ok: false,
      error: 'invalid_email',
    });
    expect(joinWaitlist).not.toHaveBeenCalled();
  });

  it('trims the email before sending it', async () => {
    joinWaitlist.mockResolvedValue({ joined: true, position: 1 });
    await joinWaitlistAction(fd('  a@example.com  '));
    expect(joinWaitlist).toHaveBeenCalledWith({
      email: 'a@example.com',
      source: undefined,
    });
  });

  it('passes the source through for channel attribution', async () => {
    joinWaitlist.mockResolvedValue({ joined: true, position: 1 });
    await joinWaitlistAction(fd('a@example.com', 'reddit'));
    expect(joinWaitlist).toHaveBeenCalledWith({
      email: 'a@example.com',
      source: 'reddit',
    });
  });

  it('sends no source rather than an empty string', async () => {
    joinWaitlist.mockResolvedValue({ joined: true, position: 1 });
    await joinWaitlistAction(fd('a@example.com', ''));
    expect(joinWaitlist).toHaveBeenCalledWith({
      email: 'a@example.com',
      source: undefined,
    });
  });

  it('surfaces a failure rather than pretending it worked', async () => {
    // Throw from inside an async fn rather than mockRejectedValue or
    // `() => Promise.reject(...)`. Both of those construct the rejected
    // promise at setup time, which vitest's unhandled-rejection tracker
    // flags before the action's try/catch ever awaits it — the test then
    // "fails" with the very error the action handled correctly.
    joinWaitlist.mockImplementation(async () => {
      throw new Error('boom');
    });
    const out = await joinWaitlistAction(fd('a@example.com'));
    expect(out).toEqual({ ok: false, error: 'join_failed' });
  });
});
