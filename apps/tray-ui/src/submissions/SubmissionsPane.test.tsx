import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { invoke } from '@tauri-apps/api/core';
import { SubmissionsPane } from './SubmissionsPane';

const mockedInvoke = vi.mocked(invoke);

const example = {
  id: 'row_1',
  raw_line: '<MissionEnded> mission ended for player',
  timestamp: '2026-05-17T12:00:00Z',
  shell_tag: 'MissionEnded',
  partial_structured: { mission: 'm1' },
  context_before: ['before line'],
  context_after: ['after line'],
  game_build: null,
  channel: 'ptu' as const,
  interest_score: 70,
  shape_hash: 'sh_mission',
  occurrence_count: 3,
  first_seen: '2026-05-17T12:00:00Z',
  last_seen: '2026-05-17T12:00:00Z',
  detected_pii: [],
  dismissed: false,
};

const groups = {
  featured: [
    {
      shell_tag: 'MissionEnded',
      shapes: 4,
      occurrences: 12,
      last_seen: '2026-05-17T12:00:00Z',
      max_interest: 70,
      example,
    },
  ],
  other: [
    {
      shell_tag: 'InventoryManagement',
      shapes: 111713,
      occurrences: 447724,
      last_seen: '2026-05-17T12:00:00Z',
      max_interest: 55,
    },
    {
      shell_tag: 'EOS_Logging',
      shapes: 149,
      occurrences: 6769,
      last_seen: '2026-05-17T12:00:00Z',
      max_interest: 55,
    },
  ],
};

let calls: Array<{ cmd: string; args: unknown }>;
let ignored: Array<{ shell_tag: string; ignored_at: string }>;

function stub() {
  mockedInvoke.mockImplementation(async (cmd: string, args?: unknown) => {
    calls.push({ cmd, args });
    switch (cmd) {
      case 'list_review_groups':
        return groups;
      case 'list_ignored_review_groups':
        return ignored;
      case 'ignore_review_groups':
        return (args as { shell_tags: string[] }).shell_tags.length;
      case 'unignore_review_group':
        return true;
      case 'review_group_example':
        return { ...example, shell_tag: 'EOS_Logging', raw_line: '<EOS_Logging> x' };
      case 'submit_unknown_lines':
        return { accepted: 1, deduped: 0, ids: ['1'] };
      default:
        return null;
    }
  });
}

beforeEach(() => {
  calls = [];
  ignored = [];
  mockedInvoke.mockReset();
  stub();
});

async function openFeatured() {
  const row = (await screen.findAllByTestId('review-group'))[0];
  await userEvent.click(within(row).getByRole('button', { name: 'Review' }));
  return row;
}

describe('SubmissionsPane', () => {
  it('shows featured groups open and Other collapsed', async () => {
    render(<SubmissionsPane />);
    expect(await screen.findByText('<MissionEnded>')).toBeInTheDocument();
    const other = screen.getByTestId('review-other');
    // Collapsed by default: the chatter is there but not open.
    expect(other).not.toHaveAttribute('open');
    expect(within(other).getByText(/2 groups, 454,493 lines/)).toBeInTheDocument();
  });

  it('reports the featured count, not raw shapes, to the badge', async () => {
    const onCountChange = vi.fn();
    render(<SubmissionsPane onCountChange={onCountChange} />);
    await waitFor(() => expect(onCountChange).toHaveBeenCalledWith(1));
  });

  it('ignores selected groups in one call', async () => {
    render(<SubmissionsPane />);
    await screen.findByText('<MissionEnded>');
    await userEvent.click(screen.getByLabelText('Select InventoryManagement'));
    await userEvent.click(screen.getByLabelText('Select EOS_Logging'));
    await userEvent.click(screen.getByRole('button', { name: 'Ignore selected (2)' }));
    await waitFor(() =>
      expect(calls.find((c) => c.cmd === 'ignore_review_groups')?.args).toEqual({
        shell_tags: ['InventoryManagement', 'EOS_Logging'],
      }),
    );
  });

  it('ignore all other asks first and sends every Other tag', async () => {
    const confirm = vi.spyOn(window, 'confirm').mockReturnValue(true);
    render(<SubmissionsPane />);
    await screen.findByText('<MissionEnded>');
    await userEvent.click(screen.getByRole('button', { name: 'Ignore all other' }));
    expect(confirm).toHaveBeenCalled();
    await waitFor(() =>
      expect(calls.find((c) => c.cmd === 'ignore_review_groups')?.args).toEqual({
        shell_tags: ['InventoryManagement', 'EOS_Logging'],
      }),
    );
    confirm.mockRestore();
  });

  it('declining the confirm ignores nothing', async () => {
    const confirm = vi.spyOn(window, 'confirm').mockReturnValue(false);
    render(<SubmissionsPane />);
    await screen.findByText('<MissionEnded>');
    await userEvent.click(screen.getByRole('button', { name: 'Ignore all other' }));
    expect(calls.some((c) => c.cmd === 'ignore_review_groups')).toBe(false);
    confirm.mockRestore();
  });

  it('an ignored group can be restored', async () => {
    ignored = [{ shell_tag: 'WebRTC/Janus', ignored_at: '2026-09-26 12:00:00' }];
    render(<SubmissionsPane />);
    const section = await screen.findByTestId('review-ignored');
    await userEvent.click(within(section).getByRole('button', { name: 'Restore' }));
    await waitFor(() =>
      expect(calls.find((c) => c.cmd === 'unignore_review_group')?.args).toEqual({
        shell_tag: 'WebRTC/Janus',
      }),
    );
  });

  it('an Other group loads its example on demand', async () => {
    render(<SubmissionsPane />);
    await screen.findByText('<MissionEnded>');
    const rows = screen.getAllByTestId('review-group');
    await userEvent.click(within(rows[2]).getByRole('button', { name: 'Review' }));
    await waitFor(() =>
      expect(calls.find((c) => c.cmd === 'review_group_example')?.args).toEqual({
        shell_tag: 'EOS_Logging',
      }),
    );
  });

  it('submits the group example with the row fields the form does not show', async () => {
    render(<SubmissionsPane />);
    await openFeatured();
    await userEvent.click(screen.getByRole('radio', { name: /anonymous/i }));
    await userEvent.click(screen.getByRole('button', { name: /submit/i }));
    await waitFor(() =>
      expect(mockedInvoke).toHaveBeenCalledWith(
        'submit_unknown_lines',
        expect.objectContaining({
          payloads: [
            expect.objectContaining({
              shape_hash: 'sh_mission',
              raw_examples: ['<MissionEnded> mission ended for player'],
              shell_tag: 'MissionEnded',
              channel: 'ptu',
              partial_structured: { mission: 'm1' },
              context_examples: [{ before: ['before line'], after: ['after line'] }],
              attributed: false,
            }),
          ],
        }),
      ),
    );
  });

  it('carries attributed: true when the user attributes', async () => {
    render(<SubmissionsPane handle="Daisy" />);
    await openFeatured();
    await userEvent.click(screen.getByRole('radio', { name: /Attribute to @Daisy/ }));
    await userEvent.click(screen.getByRole('button', { name: /submit/i }));
    await waitFor(() =>
      expect(mockedInvoke).toHaveBeenCalledWith(
        'submit_unknown_lines',
        expect.objectContaining({
          payloads: [expect.objectContaining({ attributed: true })],
        }),
      ),
    );
  });
});
