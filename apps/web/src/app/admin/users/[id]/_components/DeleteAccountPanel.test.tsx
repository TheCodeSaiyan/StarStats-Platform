import React from 'react';
import { describe, expect, it, vi } from 'vitest';
import { render, screen } from '@testing-library/react';
import { DeleteAccountPanel } from './DeleteAccountPanel';

const noop = vi.fn();

function renderPanel(isAdmin = true, handle = 'TheCodeSaiyan') {
  return render(
    <DeleteAccountPanel
      handle={handle}
      deleteAction={noop}
      isAdmin={isAdmin}
    />,
  );
}

describe('DeleteAccountPanel', () => {
  // Purge is irreversible and breaks recipients' timelines. It must be
  // CHOSEN, never landed on by clicking through.
  it('defaults to pseudonymise, not purge', () => {
    renderPanel();
    const pseudo = screen.getByRole('radio', {
      name: /pseudonymise/i,
    }) as HTMLInputElement;
    const purge = screen.getByRole('radio', { name: /purge/i }) as HTMLInputElement;
    expect(pseudo.checked).toBe(true);
    expect(purge.checked).toBe(false);
  });

  it('says what pseudonymise keeps, not just what it removes', () => {
    // An admin choosing between two irreversible-sounding options needs
    // to know that one of them preserves recipients' timelines.
    renderPanel();
    expect(
      screen.getByText(/event rows are kept but unlinked/i),
    ).toBeInTheDocument();
  });

  it('says that purge deletes their events permanently', () => {
    renderPanel();
    expect(
      screen.getByText(/deletes their events outright/i),
    ).toBeInTheDocument();
  });

  it('warns that purge breaks timelines for people they shared with', () => {
    // This is the consequence that falls on OTHER users, and the one an
    // admin is least likely to think of. Matched on the specific
    // outcome rather than "shared with", which appears in BOTH mode
    // descriptions — the loose version passed while asserting nothing
    // about purge in particular.
    renderPanel();
    expect(
      screen.getByText(/loses those rows from their own timelines/i),
    ).toBeInTheDocument();
  });

  it('requires the exact handle to be typed', () => {
    renderPanel(true, 'TheCodeSaiyan');
    const input = screen.getByLabelText(/type .* to confirm/i) as HTMLInputElement;
    expect(input).toBeRequired();
    expect(input.placeholder).toBe('TheCodeSaiyan');
  });

  it('names the account in the confirm dialog', () => {
    const confirmSpy = vi.spyOn(window, 'confirm').mockReturnValue(false);
    renderPanel(true, 'BadActor');
    screen.getByRole('button', { name: /delete account/i }).click();
    expect(confirmSpy).toHaveBeenCalledWith(
      expect.stringContaining('BadActor'),
    );
    confirmSpy.mockRestore();
  });

  it('hides the whole panel from a non-admin', () => {
    renderPanel(false);
    expect(screen.getByText(/admin role required/i)).toBeInTheDocument();
    expect(screen.queryByRole('button', { name: /delete account/i })).toBeNull();
    // And no way to smuggle a mode selection through either.
    expect(screen.queryByRole('radio')).toBeNull();
  });
});
