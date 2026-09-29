import { describe, it, expect } from 'vitest';
import { signInDestination } from './sign-in-destination';

describe('signInDestination', () => {
  it('lands in chat when asked to', () => {
    expect(signInDestination('/chat')).toBe('/chat');
  });

  it.each([
    [undefined],
    [null],
    [''],
    ['/admin'],
    ['/chat/../admin'],
    ['//evil.example/chat'],
    ['https://evil.example/chat'],
    ['/chat?x=1'],
    [['/chat']],
  ])('falls back to /me for %j', (next) => {
    expect(signInDestination(next)).toBe('/me');
  });
});
