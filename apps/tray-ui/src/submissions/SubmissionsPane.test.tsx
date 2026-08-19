import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { invoke } from '@tauri-apps/api/core';
import { SubmissionsPane } from './SubmissionsPane';

const mockedInvoke = vi.mocked(invoke);

const baseRow = {
  id: 'row_1',
  raw_line: 'Jim opened the door',
  timestamp: '2026-05-17T12:00:00Z',
  shell_tag: 'Door',
  partial_structured: {},
  context_before: [],
  context_after: [],
  game_build: null,
  channel: 'live' as const,
  interest_score: 70,
  shape_hash: 'sh_a',
  occurrence_count: 3,
  first_seen: '2026-05-17T12:00:00Z',
  last_seen: '2026-05-17T12:00:00Z',
  detected_pii: [],
  dismissed: false,
};

beforeEach(() => {
  mockedInvoke.mockReset();
});

describe('SubmissionsPane', () => {
  it('lists unknown lines from the Tauri bridge', async () => {
    mockedInvoke.mockImplementation(async (cmd: string) => {
      if (cmd === 'list_unknown_lines') return [baseRow];
      return null;
    });
    render(<SubmissionsPane />);
    expect(await screen.findByText(/sh_a/)).toBeInTheDocument();
    expect(screen.getByText('Jim opened the door')).toBeInTheDocument();
  });

  it('invokes submit_unknown_lines on Submit click', async () => {
    mockedInvoke.mockImplementation(async (cmd: string) => {
      if (cmd === 'list_unknown_lines') return [baseRow];
      if (cmd === 'submit_unknown_lines')
        return { accepted: 1, deduped: 0, ids: ['1'] };
      return null;
    });
    render(<SubmissionsPane />);
    await screen.findByText(/sh_a/);
    // Forced attribution choice gates Submit — pick one first.
    await userEvent.click(screen.getByRole('radio', { name: /anonymous/i }));
    await userEvent.click(screen.getByRole('button', { name: /submit/i }));
    await waitFor(() => {
      expect(mockedInvoke).toHaveBeenCalledWith(
        'submit_unknown_lines',
        expect.objectContaining({
          payloads: expect.arrayContaining([
            expect.objectContaining({
              shape_hash: 'sh_a',
              raw_examples: ['Jim opened the door'],
              channel: 'live',
              occurrence_count: 3,
            }),
          ]),
        })
      );
    });
  });

  it('preserves the row channel on the submitted payload', async () => {
    // Bug 1 regression: the adapter used to hardcode `'channel_live'`
    // and drop whatever the row carried. A PTU capture must surface as
    // `channel: 'ptu'` so server-side rule promotion scopes correctly.
    const ptuRow = { ...baseRow, channel: 'ptu' as const };
    mockedInvoke.mockImplementation(async (cmd: string) => {
      if (cmd === 'list_unknown_lines') return [ptuRow];
      if (cmd === 'submit_unknown_lines')
        return { accepted: 1, deduped: 0, ids: ['1'] };
      return null;
    });
    render(<SubmissionsPane />);
    await screen.findByText(/sh_a/);
    // Forced attribution choice gates Submit — pick one first.
    await userEvent.click(screen.getByRole('radio', { name: /anonymous/i }));
    await userEvent.click(screen.getByRole('button', { name: /submit/i }));
    await waitFor(() => {
      expect(mockedInvoke).toHaveBeenCalledWith(
        'submit_unknown_lines',
        expect.objectContaining({
          payloads: expect.arrayContaining([
            expect.objectContaining({ channel: 'ptu' }),
          ]),
        })
      );
    });
  });

  it('preserves context_before / context_after on the submitted payload', async () => {
    // Bug 3 regression: the adapter used to drop the surrounding-line
    // context. Reviewers can't classify a shape without seeing how it
    // sat in source order — the server stores these as
    // `context_examples[0].{before,after}`.
    const contextRow = {
      ...baseRow,
      context_before: ['X', 'Y'],
      context_after: ['Z'],
    };
    mockedInvoke.mockImplementation(async (cmd: string) => {
      if (cmd === 'list_unknown_lines') return [contextRow];
      if (cmd === 'submit_unknown_lines')
        return { accepted: 1, deduped: 0, ids: ['1'] };
      return null;
    });
    render(<SubmissionsPane />);
    await screen.findByText(/sh_a/);
    // Forced attribution choice gates Submit — pick one first.
    await userEvent.click(screen.getByRole('radio', { name: /anonymous/i }));
    await userEvent.click(screen.getByRole('button', { name: /submit/i }));
    await waitFor(() => {
      expect(mockedInvoke).toHaveBeenCalledWith(
        'submit_unknown_lines',
        expect.objectContaining({
          payloads: expect.arrayContaining([
            expect.objectContaining({
              context_examples: [{ before: ['X', 'Y'], after: ['Z'] }],
            }),
          ]),
        })
      );
    });
  });

  it('preserves partial_structured on the submitted payload', async () => {
    // Bug 3 regression: the adapter used to drop `partial_structured`,
    // which carries the captured key=value tail of shell-tagged lines
    // (`<Door> who=Jim id=42` → `{who: "Jim", id: "42"}`). Without it
    // the server-side reviewer loses the only structured hint the
    // capture pipeline produced.
    const psRow = {
      ...baseRow,
      partial_structured: { who: 'Jim', id: '42' },
    };
    mockedInvoke.mockImplementation(async (cmd: string) => {
      if (cmd === 'list_unknown_lines') return [psRow];
      if (cmd === 'submit_unknown_lines')
        return { accepted: 1, deduped: 0, ids: ['1'] };
      return null;
    });
    render(<SubmissionsPane />);
    await screen.findByText(/sh_a/);
    // Forced attribution choice gates Submit — pick one first.
    await userEvent.click(screen.getByRole('radio', { name: /anonymous/i }));
    await userEvent.click(screen.getByRole('button', { name: /submit/i }));
    await waitFor(() => {
      expect(mockedInvoke).toHaveBeenCalledWith(
        'submit_unknown_lines',
        expect.objectContaining({
          payloads: expect.arrayContaining([
            expect.objectContaining({
              partial_structured: { who: 'Jim', id: '42' },
            }),
          ]),
        })
      );
    });
  });

  it('invokes dismiss_unknown_line on Dismiss click', async () => {
    mockedInvoke.mockImplementation(async (cmd: string) => {
      if (cmd === 'list_unknown_lines') return [baseRow];
      return null;
    });
    render(<SubmissionsPane />);
    await screen.findByText(/sh_a/);
    await userEvent.click(screen.getByRole('button', { name: /dismiss/i }));
    await waitFor(() => {
      expect(mockedInvoke).toHaveBeenCalledWith('dismiss_unknown_line', {
        shape_hash: 'sh_a',
      });
    });
  });

  it('carries attributed: true into the payload when the user attributes', async () => {
    mockedInvoke.mockImplementation(async (cmd: string) => {
      if (cmd === 'list_unknown_lines') return [baseRow];
      if (cmd === 'submit_unknown_lines')
        return { accepted: 1, deduped: 0, ids: ['1'] };
      return null;
    });
    render(<SubmissionsPane handle="Daisy" />);
    await screen.findByText(/sh_a/);
    await userEvent.click(screen.getByRole('radio', { name: /attribute to @Daisy/i }));
    await userEvent.click(screen.getByRole('button', { name: /submit/i }));
    await waitFor(() => {
      expect(mockedInvoke).toHaveBeenCalledWith(
        'submit_unknown_lines',
        expect.objectContaining({
          payloads: expect.arrayContaining([
            expect.objectContaining({ attributed: true }),
          ]),
        })
      );
    });
  });

  it('carries attributed: false into the payload when the user stays anonymous', async () => {
    mockedInvoke.mockImplementation(async (cmd: string) => {
      if (cmd === 'list_unknown_lines') return [baseRow];
      if (cmd === 'submit_unknown_lines')
        return { accepted: 1, deduped: 0, ids: ['1'] };
      return null;
    });
    render(<SubmissionsPane />);
    await screen.findByText(/sh_a/);
    await userEvent.click(screen.getByRole('radio', { name: /anonymous/i }));
    await userEvent.click(screen.getByRole('button', { name: /submit/i }));
    await waitFor(() => {
      expect(mockedInvoke).toHaveBeenCalledWith(
        'submit_unknown_lines',
        expect.objectContaining({
          payloads: expect.arrayContaining([
            expect.objectContaining({ attributed: false }),
          ]),
        })
      );
    });
  });

  it('reports count changes to the parent via onCountChange', async () => {
    mockedInvoke.mockImplementation(async (cmd: string) => {
      if (cmd === 'list_unknown_lines') return [baseRow, { ...baseRow, shape_hash: 'sh_b' }];
      return null;
    });
    const onCountChange = vi.fn();
    render(<SubmissionsPane onCountChange={onCountChange} />);
    await waitFor(() => expect(onCountChange).toHaveBeenCalledWith(2));
  });
});
