import React from 'react';
import { describe, expect, it, vi } from 'vitest';
import { render, screen } from '@testing-library/react';
import { RestrictionPanel } from './RestrictionPanel';
import type { AdminRestrictionDto } from '@/lib/api';

const noop = vi.fn();

function restriction(
  overrides: Partial<AdminRestrictionDto> = {},
): AdminRestrictionDto {
  return {
    ingest_blocked: true,
    sharing_blocked: true,
    public_profile_blocked: true,
    submissions_blocked: true,
    reason: 'harassment',
    restricted_by: 'modhandle',
    restricted_at: '2026-08-11T12:00:00Z',
    expires_at: null,
    is_suspension: true,
    shares_revoked: 3,
    ...overrides,
  } as AdminRestrictionDto;
}

function renderPanel(current: AdminRestrictionDto | null, canModerate = true) {
  return render(
    <RestrictionPanel
      current={current}
      restrictAction={noop}
      reinstateAction={noop}
      canModerate={canModerate}
    />,
  );
}

describe('RestrictionPanel', () => {
  // The single most important thing this UI must communicate. A
  // suspension DELETES share grants in SpiceDB; lifting the restriction
  // does not bring them back. A moderator who reads Reinstate as an
  // undo will reach for suspension more freely than they should.
  it('states that reinstating does not restore revoked shares', () => {
    renderPanel(restriction());
    expect(
      screen.getByText(/revoked shares are not restored/i),
    ).toBeInTheDocument();
  });

  it('warns about irreversible share revocation before suspending', () => {
    // The warning lives in the native confirm() the button gates on,
    // so the only way to assert it is to spy on the call. Asserting
    // that "a confirm exists" would pass on a dialog that said nothing
    // about the shares being unrecoverable.
    const confirmSpy = vi
      .spyOn(window, 'confirm')
      .mockReturnValue(false);
    renderPanel(null);

    screen
      .getByRole('button', { name: /suspend \(all capabilities\)/i })
      .click();

    expect(confirmSpy).toHaveBeenCalledWith(
      expect.stringContaining('NOT restored'),
    );
    confirmSpy.mockRestore();
  });

  it('distinguishes a suspension from a targeted limit', () => {
    renderPanel(restriction());
    expect(screen.getByText('Suspended')).toBeInTheDocument();

    renderPanel(
      restriction({
        ingest_blocked: false,
        public_profile_blocked: false,
        submissions_blocked: false,
        is_suspension: false,
      }),
    );
    expect(screen.getByText('Limited')).toBeInTheDocument();
  });

  it('shows the reason and who set it', () => {
    renderPanel(restriction());
    expect(screen.getByText('harassment')).toBeInTheDocument();
    expect(screen.getByText('modhandle')).toBeInTheDocument();
  });

  it('renders a null expiry as "until lifted", never as a date', () => {
    renderPanel(restriction({ expires_at: null }));
    expect(screen.getByText('until lifted')).toBeInTheDocument();
  });

  it('renders a real expiry as a date', () => {
    renderPanel(restriction({ expires_at: '2026-09-01T00:00:00Z' }));
    expect(screen.getByText('2026-09-01')).toBeInTheDocument();
  });

  it('says plainly what an unrestricted account can still do', () => {
    renderPanel(null);
    expect(screen.getByText(/no restrictions/i)).toBeInTheDocument();
  });

  it('names the consequence of each capability, not just its label', () => {
    // "Public profile" alone does not tell a moderator that the user's
    // public pages stop serving, which is the part that matters.
    renderPanel(null);
    expect(
      screen.getByText(/hidden from \/discover/i),
    ).toBeInTheDocument();
    expect(
      screen.getByText(/tray can no longer upload/i),
    ).toBeInTheDocument();
  });

  it('hides the controls from a non-moderator', () => {
    renderPanel(restriction(), false);
    expect(
      screen.getByText(/moderator role required/i),
    ).toBeInTheDocument();
    expect(
      screen.queryByRole('button', { name: /suspend/i }),
    ).toBeNull();
  });

  it('pre-checks the capabilities already blocked', () => {
    renderPanel(
      restriction({
        ingest_blocked: false,
        sharing_blocked: true,
        public_profile_blocked: false,
        submissions_blocked: false,
        is_suspension: false,
      }),
    );
    const boxes = screen.getAllByRole('checkbox') as HTMLInputElement[];
    const byName = Object.fromEntries(boxes.map((b) => [b.name, b.checked]));
    expect(byName.sharing_blocked).toBe(true);
    expect(byName.ingest_blocked).toBe(false);
  });
});
