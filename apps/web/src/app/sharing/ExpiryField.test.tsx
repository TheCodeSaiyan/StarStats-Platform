import React from 'react';
import { describe, it, expect } from 'vitest';
import { render, screen, fireEvent } from '@testing-library/react';
import { ExpiryField } from './ExpiryField';

/**
 * The prefill used to be copied into state by an effect after mount. It is
 * now a render-time adjustment keyed on the prop (the React "adjust state
 * when a prop changes" pattern), and that keying is the whole contract:
 * apply a prefill once, never over a user's own edit, and take a NEW prefill
 * when the prop changes. Removing the key — applying on every render — fails
 * the third case; never applying fails the first.
 */
const LOCAL = /^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}$/;
const field = () =>
  screen.getByLabelText('Auto-expiry (optional)') as HTMLInputElement;

describe('ExpiryField prefill', () => {
  it('lands the localised prefill and the browser offset on the client', () => {
    const { container } = render(<ExpiryField prefillIso="2026-09-20T12:30:00Z" />);
    expect(field().value).toMatch(LOCAL);
    const tz = container.querySelector<HTMLInputElement>('input[name="tz_offset_minutes"]');
    expect(tz?.value).toBe(String(new Date().getTimezoneOffset()));
  });

  it('starts empty without a prefill', () => {
    render(<ExpiryField />);
    expect(field().value).toBe('');
  });

  it("keeps the user's edit across a rerender with the same prefill", () => {
    const { rerender } = render(<ExpiryField prefillIso="2026-09-20T12:30:00Z" />);
    fireEvent.change(field(), { target: { value: '2027-01-01T09:00' } });
    expect(field().value).toBe('2027-01-01T09:00');
    rerender(<ExpiryField prefillIso="2026-09-20T12:30:00Z" />);
    expect(field().value).toBe('2027-01-01T09:00');
  });

  it('takes a new prefill when the prop changes', () => {
    const { rerender } = render(<ExpiryField prefillIso="2026-09-20T12:30:00Z" />);
    const first = field().value;
    rerender(<ExpiryField prefillIso="2026-12-25T08:00:00Z" />);
    expect(field().value).toMatch(LOCAL);
    expect(field().value).not.toBe(first);
  });
});
