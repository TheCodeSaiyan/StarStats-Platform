import React from 'react';
import { describe, it, expect, vi, afterEach } from 'vitest';
import {
  cleanup,
  fireEvent,
  render,
  screen,
  waitFor,
} from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import type { WaitlistEntryApi, WaitlistConfigApi } from '@/lib/api';

const admitAction = vi.fn();
const resendAction = vi.fn();
const saveConfigAction = vi.fn();
const deleteAction = vi.fn();
vi.mock('@/app/_actions/waitlist-admin', () => ({
  admitWaitlistAction: (...a: unknown[]) => admitAction(...a),
  resendWaitlistAction: (...a: unknown[]) => resendAction(...a),
  saveWaitlistConfigAction: (...a: unknown[]) => saveConfigAction(...a),
  deleteWaitlistAction: (...a: unknown[]) => deleteAction(...a),
}));

import { WaitlistConsole } from './WaitlistConsole';

afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
  admitAction.mockReset();
  resendAction.mockReset();
  saveConfigAction.mockReset();
  deleteAction.mockReset();
});

const config: WaitlistConfigApi = { cap: 50, gate_enabled: true };

const admittedRow: WaitlistEntryApi = {
  id: 'row-1',
  email: 'ntatschner@gmail.com',
  source: 'beta-gate',
  created_at: '2026-07-23T15:35:00Z',
  admitted_at: '2026-07-23T15:35:00Z',
};

const queuedRow: WaitlistEntryApi = {
  id: 'row-2',
  email: 'pending@example.com',
  source: null,
  created_at: '2026-07-24T10:00:00Z',
};

const renderConsole = (over: Partial<React.ComponentProps<typeof WaitlistConsole>> = {}) =>
  render(
    <WaitlistConsole
      queued={[]}
      admitted={[admittedRow]}
      admittedCount={1}
      config={config}
      {...over}
    />,
  );

describe('WaitlistConsole admitted table + resend', () => {
  it('lists admitted rows instead of only counting them', () => {
    renderConsole();
    // The row an outage stranded must be visible, not just a number.
    expect(screen.getByText('ntatschner@gmail.com')).toBeTruthy();
  });

  it('resends the selected admitted row and reports success', async () => {
    resendAction.mockImplementation(async () => ({ ok: true, resent: 1 }));
    renderConsole();
    fireEvent.click(
      screen.getByLabelText('Select ntatschner@gmail.com to resend'),
    );
    fireEvent.click(screen.getByRole('button', { name: /resend/i }));
    await waitFor(() => expect(resendAction).toHaveBeenCalledWith(['row-1']));
    await waitFor(() =>
      expect(screen.getByText(/re-sent 1 invite/i)).toBeTruthy(),
    );
  });

  it('does NOT claim success when the transport is still failing', async () => {
    // resent < asked: a partial/zero send must read as a failure, not a
    // green chip — the whole point of counting successful sends.
    resendAction.mockImplementation(async () => ({ ok: true, resent: 0 }));
    renderConsole();
    fireEvent.click(
      screen.getByLabelText('Select ntatschner@gmail.com to resend'),
    );
    fireEvent.click(screen.getByRole('button', { name: /resend/i }));
    await waitFor(() =>
      expect(screen.getByText(/failed to send/i)).toBeTruthy(),
    );
    expect(screen.queryByText(/re-sent 1 invite/i)).toBeNull();
  });

  it('shows an empty state when nobody is admitted', () => {
    renderConsole({ admitted: [], admittedCount: 0 });
    expect(screen.getByText(/nobody has been admitted yet/i)).toBeTruthy();
  });
});

describe('WaitlistConsole delete selected (queue table)', () => {
  // The queue's delete button reads "Delete N from queue" — counted and
  // qualified, distinct from the admitted table's "Delete N admitted" —
  // so these no longer need to render with the other table empty to avoid
  // a `getByRole` collision. Kept isolated anyway to keep each test
  // focused; see the "both tables at once" describe block below for the
  // regression coverage that isolation used to make impossible.
  it('deletes the selected rows and names the one refused as already redeemed', async () => {
    // `blocked` is designed as ids so the console can say WHICH row
    // survived, not just how many — collapsing to a count was the bug.
    const redeemedRow: WaitlistEntryApi = {
      id: 'row-3',
      email: 'redeemed@example.com',
      source: null,
      created_at: '2026-07-24T10:00:00Z',
    };
    deleteAction.mockImplementation(async () => ({
      ok: true,
      deleted: [queuedRow.id],
      blocked: [redeemedRow.id],
    }));
    vi.spyOn(window, 'confirm').mockReturnValue(true);
    renderConsole({
      queued: [queuedRow, redeemedRow],
      admitted: [],
      admittedCount: 0,
    });

    fireEvent.click(screen.getByLabelText(`Select ${queuedRow.email}`));
    fireEvent.click(screen.getByLabelText(`Select ${redeemedRow.email}`));
    fireEvent.click(
      screen.getByRole('button', { name: /delete 2 from queue/i }),
    );

    await waitFor(() =>
      expect(deleteAction).toHaveBeenCalledWith([queuedRow.id, redeemedRow.id]),
    );
    await waitFor(() =>
      expect(
        screen.getByText(
          /deleted 1\. skipped 1 already redeemed: redeemed@example\.com\./i,
        ),
      ).toBeTruthy(),
    );
  });

  it('does not delete when the confirm is dismissed', async () => {
    vi.spyOn(window, 'confirm').mockReturnValue(false);
    renderConsole({ queued: [queuedRow], admitted: [], admittedCount: 0 });

    fireEvent.click(screen.getByLabelText(`Select ${queuedRow.email}`));
    fireEvent.click(
      screen.getByRole('button', { name: /delete 1 from queue/i }),
    );

    await waitFor(() => expect(window.confirm).toHaveBeenCalled());
    expect(deleteAction).not.toHaveBeenCalled();
  });

  it('reports rows that vanished out from under the request as neither deleted nor blocked', async () => {
    // Three selected, but only one comes back accounted for (none
    // blocked) — e.g. another moderator, or a stale tab, already deleted
    // the rest. `deleted: 1` alone would silently drop two rows.
    deleteAction.mockImplementation(async () => ({
      ok: true,
      deleted: [queuedRow.id],
      blocked: [],
    }));
    vi.spyOn(window, 'confirm').mockReturnValue(true);
    const extraRow: WaitlistEntryApi = {
      id: 'row-3',
      email: 'third@example.com',
      source: null,
      created_at: '2026-07-24T10:00:00Z',
    };
    const anotherRow: WaitlistEntryApi = {
      id: 'row-4',
      email: 'fourth@example.com',
      source: null,
      created_at: '2026-07-24T10:00:00Z',
    };
    renderConsole({
      queued: [queuedRow, extraRow, anotherRow],
      admitted: [],
      admittedCount: 0,
    });

    fireEvent.click(screen.getByLabelText(`Select ${queuedRow.email}`));
    fireEvent.click(screen.getByLabelText(`Select ${extraRow.email}`));
    fireEvent.click(screen.getByLabelText(`Select ${anotherRow.email}`));
    fireEvent.click(
      screen.getByRole('button', { name: /delete 3 from queue/i }),
    );

    await waitFor(() =>
      expect(
        screen.getByText(/deleted 1 of 3 — the rest were already removed\./i),
      ).toBeTruthy(),
    );
  });
});

describe('WaitlistConsole delete selected (admitted table)', () => {
  // The maintainer's throwaway test addresses (ntatschner+bacon@ etc.) show
  // up as ADMITTED rows — they were admitted, then their invite resends
  // failed in production. Deleting them requires a delete path off
  // `resendSelected`, not just the queue's `selected`. Default props here
  // already isolate to just the Admitted table (queued: []).
  it('deletes the selected admitted row and clears its selection', async () => {
    deleteAction.mockImplementation(async () => ({
      ok: true,
      deleted: [admittedRow.id],
      blocked: [],
    }));
    vi.spyOn(window, 'confirm').mockReturnValue(true);
    renderConsole();

    fireEvent.click(
      screen.getByLabelText('Select ntatschner@gmail.com to resend'),
    );
    fireEvent.click(
      screen.getByRole('button', { name: /delete 1 admitted/i }),
    );

    await waitFor(() =>
      expect(deleteAction).toHaveBeenCalledWith([admittedRow.id]),
    );
    await waitFor(() =>
      expect(screen.getByText(/deleted 1\./i)).toBeTruthy(),
    );
    // Selection cleared after a successful delete — the checkbox itself
    // stays rendered (this test doesn't re-fetch), but unchecked.
    expect(
      screen.getByLabelText('Select ntatschner@gmail.com to resend'),
    ).not.toBeChecked();
  });

  it('does not delete the admitted row when the confirm is dismissed', async () => {
    vi.spyOn(window, 'confirm').mockReturnValue(false);
    renderConsole();

    fireEvent.click(
      screen.getByLabelText('Select ntatschner@gmail.com to resend'),
    );
    fireEvent.click(
      screen.getByRole('button', { name: /delete 1 admitted/i }),
    );

    await waitFor(() => expect(window.confirm).toHaveBeenCalled());
    expect(deleteAction).not.toHaveBeenCalled();
  });
});

describe('WaitlistConsole delete selected (both tables at once)', () => {
  // Regression coverage for the crossed-wiring this refactor could have
  // introduced. Before the two Delete buttons had distinct accessible
  // names ("Delete N from queue" vs "Delete N admitted"), rendering both
  // tables populated at once made `getByRole('button', { name: /delete
  // selected/i })` match both and throw — so this scenario was untestable.
  // Now that the names are disambiguated, assert each button submits ONLY
  // its own table's selection.
  it("targets each table's own selection set, never the other one", async () => {
    deleteAction.mockImplementation(async (ids: string[]) => ({
      ok: true,
      deleted: ids,
      blocked: [],
    }));
    vi.spyOn(window, 'confirm').mockReturnValue(true);
    renderConsole({
      queued: [queuedRow],
      admitted: [admittedRow],
      admittedCount: 1,
    });

    fireEvent.click(screen.getByLabelText(`Select ${queuedRow.email}`));
    fireEvent.click(
      screen.getByRole('button', { name: /delete 1 from queue/i }),
    );
    await waitFor(() =>
      expect(deleteAction).toHaveBeenCalledWith([queuedRow.id]),
    );
    expect(deleteAction).not.toHaveBeenCalledWith([admittedRow.id]);

    deleteAction.mockClear();
    fireEvent.click(
      screen.getByLabelText('Select ntatschner@gmail.com to resend'),
    );
    fireEvent.click(
      screen.getByRole('button', { name: /delete 1 admitted/i }),
    );
    await waitFor(() =>
      expect(deleteAction).toHaveBeenCalledWith([admittedRow.id]),
    );
    expect(deleteAction).not.toHaveBeenCalledWith([queuedRow.id]);
  });
});

describe('WaitlistConsole delete notice — naming blocked rows (F1)', () => {
  it('caps the named blocked addresses in a large batch rather than listing them all', async () => {
    const rows: WaitlistEntryApi[] = ['a', 'b', 'c', 'd', 'e'].map((letter) => ({
      id: `blocked-${letter}`,
      email: `${letter}@example.com`,
      source: null,
      created_at: '2026-07-20T10:00:00Z',
      admitted_at: '2026-07-20T10:05:00Z',
    }));
    deleteAction.mockImplementation(async () => ({
      ok: true,
      deleted: [],
      blocked: rows.map((r) => r.id),
    }));
    vi.spyOn(window, 'confirm').mockReturnValue(true);
    renderConsole({ admitted: rows, admittedCount: rows.length });

    rows.forEach((r) =>
      fireEvent.click(screen.getByLabelText(`Select ${r.email} to resend`)),
    );
    fireEvent.click(
      screen.getByRole('button', { name: /delete 5 admitted/i }),
    );

    await waitFor(() =>
      expect(
        screen.getByText(
          /skipped 5 already redeemed: a@example\.com, b@example\.com, c@example\.com, \+2 more\./i,
        ),
      ).toBeTruthy(),
    );
  });
});

describe('WaitlistConsole redeemed-row protection (F2)', () => {
  // `admin_list` now selects `invite_consumed_at`: a row whose invite was
  // already redeemed (an account exists) is refused by `delete_batch`
  // regardless, but rendering it identically to a deletable row makes a
  // correct refusal look like a dead button. This badges the row and
  // disables its (shared resend/delete) checkbox instead.
  const redeemedRow: WaitlistEntryApi = {
    id: 'row-6',
    email: 'used@example.com',
    source: null,
    created_at: '2026-07-20T10:00:00Z',
    admitted_at: '2026-07-20T10:05:00Z',
    invite_consumed_at: '2026-07-23T16:00:00Z',
  };

  it('badges a redeemed row and its checkbox cannot be selected', async () => {
    renderConsole({ admitted: [redeemedRow], admittedCount: 1 });

    expect(screen.getByText('Redeemed')).toBeTruthy();
    const checkbox = screen.getByLabelText(
      /used@example\.com — already redeemed/i,
    );
    expect(checkbox).toBeDisabled();

    // `fireEvent.click` bypasses jsdom's (incomplete) enforcement of
    // "disabled elements ignore interaction", so use `userEvent`, which
    // honours it the way a real click in a browser would — nothing
    // happens, same as an admin clicking this checkbox for real.
    const user = userEvent.setup();
    await user.click(checkbox);
    expect(checkbox).not.toBeChecked();
  });

  it('a non-redeemed admitted row has no badge and stays selectable', () => {
    renderConsole();
    expect(screen.queryByText('Redeemed')).toBeNull();
    expect(
      screen.getByLabelText('Select ntatschner@gmail.com to resend'),
    ).not.toBeDisabled();
  });
});
