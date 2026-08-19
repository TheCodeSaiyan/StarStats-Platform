import { describe, it, expect } from 'vitest';
import { composeProfileUrl } from '../profileUrl';

describe('composeProfileUrl', () => {
  it('returns null when both inputs are null', () => {
    expect(composeProfileUrl(null, null)).toBeNull();
  });

  it('returns null when both inputs are empty strings', () => {
    expect(composeProfileUrl('', '')).toBeNull();
  });

  it('returns null when only the origin is null', () => {
    expect(composeProfileUrl(null, 'alice')).toBeNull();
  });

  it('returns null when only the handle is null', () => {
    expect(composeProfileUrl('https://example.com', null)).toBeNull();
  });

  it('returns null when only the origin is empty', () => {
    expect(composeProfileUrl('', 'alice')).toBeNull();
  });

  it('returns null when only the handle is empty', () => {
    expect(composeProfileUrl('https://example.com', '')).toBeNull();
  });

  it('composes a clean URL when both are set', () => {
    expect(composeProfileUrl('https://example.com', 'alice')).toBe(
      'https://example.com/u/alice',
    );
  });

  it('trims a single trailing slash from the origin', () => {
    expect(composeProfileUrl('https://example.com/', 'alice')).toBe(
      'https://example.com/u/alice',
    );
  });

  it('trims multiple trailing slashes from the origin', () => {
    expect(composeProfileUrl('https://example.com///', 'alice')).toBe(
      'https://example.com/u/alice',
    );
  });

  it('URI-encodes the handle', () => {
    expect(composeProfileUrl('https://example.com', 'alice bob')).toBe(
      'https://example.com/u/alice%20bob',
    );
  });

  it('URI-encodes special characters in the handle', () => {
    expect(composeProfileUrl('https://example.com', 'a/b?c')).toBe(
      'https://example.com/u/a%2Fb%3Fc',
    );
  });
});
