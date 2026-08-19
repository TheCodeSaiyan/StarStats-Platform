/**
 * Vitest spec for the tray "What's new" pane (Phase 8 roadmap).
 *
 * Covers:
 *  - renders the 3 unread cards returned by `get_whats_new`
 *  - clicking a card invokes `mark_whats_new_seen` AND opens the
 *    public detail page via @tauri-apps/plugin-shell
 *  - empty state (zero items) shows the "All caught up" message
 *  - the More-on-web link opens `/roadmap` on click
 */

import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render, screen, waitFor, fireEvent } from '@testing-library/react';
import { invoke } from '@tauri-apps/api/core';
import {
  WhatsNewPane,
  relativeTimeSince,
  type WhatsNewItem,
  type WhatsNewResponse,
} from './WhatsNewPane';

// `vi.mock` calls are hoisted above all imports, so the mock factory
// must not reference module-level `const`s declared after it.
// `vi.hoisted` gives us a sanctioned escape hatch for sharing a fn
// between the (hoisted) mock factory and the (non-hoisted) test body.
const { openMock } = vi.hoisted(() => ({
  openMock: vi.fn(async () => {}),
}));
vi.mock('@tauri-apps/plugin-shell', () => ({
  open: openMock,
}));

const mockedInvoke = vi.mocked(invoke);

function makeItem(over: Partial<WhatsNewItem> = {}): WhatsNewItem {
  return {
    roadmap_item_id: '01963f37-3aa1-7000-8000-000000000001',
    slug: 'feature-x',
    title: 'Feature X',
    headline_status: 'shipped',
    latest_changelog_entry_id: '01963f37-3aa1-7000-8000-000000000002',
    latest_published_at: '2026-05-22T12:00:00Z',
    unread: true,
    ...over,
  };
}

interface Harness {
  whatsNewQueue: WhatsNewResponse[];
  defaultResponse: WhatsNewResponse;
  markSeenCalls: Array<{ item_id: string; entry_id: string }>;
}

function stubInvoke(harness: Harness) {
  mockedInvoke.mockImplementation((cmd: string, args?: unknown) => {
    if (cmd === 'get_whats_new') {
      const next = harness.whatsNewQueue.shift() ?? harness.defaultResponse;
      return Promise.resolve(next);
    }
    if (cmd === 'mark_whats_new_seen') {
      // Keys are byte-exact snake_case — the Rust command carries
      // `rename_all = "snake_case"` (C1 fix), so the IPC payload is
      // `{ item_id, entry_id }`, NOT camelCase.
      const a = (args ?? {}) as { item_id: string; entry_id: string };
      harness.markSeenCalls.push({ item_id: a.item_id, entry_id: a.entry_id });
      return Promise.resolve();
    }
    return Promise.reject(new Error(`unexpected invoke: ${cmd}`));
  });
}

describe('WhatsNewPane', () => {
  let harness: Harness;

  beforeEach(() => {
    mockedInvoke.mockReset();
    openMock.mockReset();
    openMock.mockImplementation(async () => {});
    harness = {
      whatsNewQueue: [],
      defaultResponse: { items: [], seen_via_auth: false },
      markSeenCalls: [],
    };
    stubInvoke(harness);
  });

  it('renders items returned by get_whats_new', async () => {
    harness.defaultResponse = {
      items: [
        makeItem({
          roadmap_item_id: 'aaa',
          slug: 'alpha',
          title: 'Alpha Feature',
        }),
        makeItem({
          roadmap_item_id: 'bbb',
          slug: 'beta',
          title: 'Beta Feature',
        }),
      ],
      seen_via_auth: true,
    };

    render(<WhatsNewPane webOrigin="https://starstats.app" />);

    expect(await screen.findByText('Alpha Feature')).toBeInTheDocument();
    expect(screen.getByText('Beta Feature')).toBeInTheDocument();
    expect(screen.queryByTestId('whatsnew-empty')).not.toBeInTheDocument();
  });

  it('shows the empty state when no items are returned', async () => {
    harness.defaultResponse = { items: [], seen_via_auth: true };

    render(<WhatsNewPane webOrigin="https://starstats.app" />);

    expect(await screen.findByTestId('whatsnew-empty')).toBeInTheDocument();
    expect(screen.getByTestId('whatsnew-empty')).toHaveTextContent(
      /all caught up/i,
    );
  });

  it('marks an item seen and opens the detail page on click', async () => {
    harness.defaultResponse = {
      items: [
        makeItem({
          roadmap_item_id: 'aaa',
          slug: 'alpha-slug',
          latest_changelog_entry_id: 'entry-1',
        }),
      ],
      seen_via_auth: true,
    };

    render(<WhatsNewPane webOrigin="https://starstats.app" />);

    const card = await screen.findByText('Feature X');
    fireEvent.click(card);

    await waitFor(() => {
      expect(harness.markSeenCalls).toHaveLength(1);
    });
    expect(harness.markSeenCalls[0]).toEqual({
      item_id: 'aaa',
      entry_id: 'entry-1',
    });
    await waitFor(() => {
      expect(openMock).toHaveBeenCalledWith(
        'https://starstats.app/roadmap/alpha-slug',
      );
    });
  });

  it('does NOT call mark_whats_new_seen on the anonymous path', async () => {
    harness.defaultResponse = {
      items: [makeItem({ roadmap_item_id: 'aaa', slug: 'alpha' })],
      seen_via_auth: false,
    };

    render(<WhatsNewPane webOrigin="https://starstats.app" />);

    const card = await screen.findByText('Feature X');
    fireEvent.click(card);

    // Wait long enough for any pending markSeen IPC to land if it
    // were going to. We only assert openShell ran (the navigation
    // affordance still works) and that no markSeen was attempted.
    await waitFor(() => {
      expect(openMock).toHaveBeenCalled();
    });
    expect(harness.markSeenCalls).toHaveLength(0);
  });

  it('opens /roadmap when "More on web" is clicked', async () => {
    harness.defaultResponse = {
      items: [makeItem()],
      seen_via_auth: true,
    };

    render(<WhatsNewPane webOrigin="https://starstats.app" />);

    await screen.findByText('Feature X');
    const more = screen.getByRole('button', { name: /more on web/i });
    fireEvent.click(more);

    await waitFor(() => {
      expect(openMock).toHaveBeenCalledWith('https://starstats.app/roadmap');
    });
  });
});

describe('relativeTimeSince', () => {
  it('returns "just now" for under a minute', () => {
    const now = new Date('2026-05-22T12:00:30Z');
    expect(relativeTimeSince('2026-05-22T12:00:00Z', now)).toBe('just now');
  });

  it('formats minutes / hours / days correctly', () => {
    const now = new Date('2026-05-22T12:00:00Z');
    expect(relativeTimeSince('2026-05-22T11:45:00Z', now)).toBe('15m ago');
    expect(relativeTimeSince('2026-05-22T09:00:00Z', now)).toBe('3h ago');
    expect(relativeTimeSince('2026-05-20T12:00:00Z', now)).toBe('2d ago');
  });
});
