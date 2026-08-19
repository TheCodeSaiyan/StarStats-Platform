import { describe, it, expect } from 'vitest';
import { friendlyError } from './friendlyError';

describe('friendlyError', () => {
  it('maps timeout strings', () => {
    const e = friendlyError(new Error('request timed out after 5s'));
    expect(e.title).toBe('Timed out');
    expect(e.body).toContain("didn't respond");
  });
  it('maps connection refused / network errors', () => {
    const e = friendlyError(new Error('connection refused: tcp 127.0.0.1:8080'));
    expect(e.title).toBe("Couldn't connect");
  });
  it('maps 401/403 to rejected', () => {
    const e = friendlyError(new Error('server returned HTTP 401'));
    expect(e.title).toBe('Rejected');
    expect(e.hint).toContain('re-pair');
  });
  it('maps 404', () => {
    const e = friendlyError(new Error('endpoint 404 not found'));
    expect(e.title).toBe('Endpoint not found');
  });
  it('maps 5xx', () => {
    const e = friendlyError(new Error('server returned HTTP 502'));
    expect(e.title).toBe('Server error');
  });
  it('maps RSI cookie failures', () => {
    const e = friendlyError(new Error('Rsi-Token cookie missing'));
    expect(e.title).toBe('No RSI cookie');
  });
  it('does not classify generic cookie text as RSI cookie', () => {
    const e = friendlyError(new Error('document.cookie is blocked by user setting'));
    expect(e.title).not.toBe('No RSI cookie');
  });
  it('falls back with 200-char cap', () => {
    const long = 'a'.repeat(300);
    const e = friendlyError(new Error(long));
    expect(e.title).toBe('Something went wrong');
    expect(e.body.endsWith('…')).toBe(true);
    expect(e.body.length).toBeLessThanOrEqual(201);
  });
  it('handles non-Error inputs', () => {
    const e = friendlyError('plain string error');
    expect(e.body).toContain('plain string error');
  });
});
