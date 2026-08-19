import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render, screen, waitFor, fireEvent, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { invoke } from '@tauri-apps/api/core';
import { LogsPane } from './LogsPane';
import type { SearchEventsResult, TimelineEntry } from '../api';

const mockedInvoke = vi.mocked(invoke);

// Counter so each manufactured TimelineEntry has a unique id without
// the tests having to thread it through every call.
let nextId = 0;

function makeEntry(over: Partial<TimelineEntry> = {}): TimelineEntry {
  nextId += 1;
  const id = over.id ?? nextId;
  return {
    id,
    timestamp: `2026-05-21T12:00:${String(id % 60).padStart(2, '0')}.000Z`,
    event_type: 'location',
    summary: `event #${id}`,
    raw_line: `raw ${id}`,
    log_source: 'live',
    synced: true,
    ...over,
  };
}

function makeResult(
  entries: TimelineEntry[],
  over: Partial<SearchEventsResult> = {},
): SearchEventsResult {
  return {
    entries,
    total: entries.length,
    has_more: false,
    ...over,
  };
}

// Capture every `search_events` invocation so the tests can assert on
// the exact arg shape (debounce coalescing, type filter, before_id).
interface SearchCall {
  query?: string;
  type_filter?: string;
  before_id?: number | null;
  limit?: number;
}

interface InvokeHarness {
  searchCalls: SearchCall[];
  searchQueue: SearchEventsResult[];
  defaultResult: SearchEventsResult;
}

function stubInvoke(harness: InvokeHarness) {
  mockedInvoke.mockImplementation((cmd: string, args?: unknown) => {
    if (cmd === 'search_events') {
      harness.searchCalls.push((args ?? {}) as SearchCall);
      const next = harness.searchQueue.shift() ?? harness.defaultResult;
      return Promise.resolve(next);
    }
    if (cmd === 'get_storage_stats') {
      return Promise.resolve({ total_events: 0, db_size_bytes: 0 });
    }
    if (cmd === 'count_quarantined') {
      return Promise.resolve(0);
    }
    if (cmd === 'list_transactions') {
      return Promise.resolve([]);
    }
    return Promise.reject(new Error(`unexpected invoke: ${cmd}`));
  });
}

describe('LogsPane', () => {
  let harness: InvokeHarness;

  beforeEach(() => {
    nextId = 0;
    mockedInvoke.mockReset();
    harness = {
      searchCalls: [],
      searchQueue: [],
      defaultResult: makeResult([]),
    };
    stubInvoke(harness);
  });

  it('fires an initial search with no query and no type filter on mount', async () => {
    harness.defaultResult = makeResult([makeEntry({ event_type: 'location' })]);
    render(<LogsPane />);

    await waitFor(() => {
      const initial = harness.searchCalls.find(
        (c) => c.query === undefined && c.type_filter === undefined && c.before_id === undefined,
      );
      expect(initial).toBeDefined();
    });
    expect(await screen.findByText(/event #1/)).toBeInTheDocument();
  });

  it('debounces the search input and dispatches the typed query', async () => {
    const user = userEvent.setup();
    harness.defaultResult = makeResult([makeEntry()]);
    render(<LogsPane />);

    // Wait for the initial fetch so the debounce timer for the first
    // search has already drained.
    await waitFor(() => expect(harness.searchCalls.length).toBeGreaterThan(0));
    const baseline = harness.searchCalls.length;

    const input = screen.getByPlaceholderText(/Filter by type or summary/i);
    await user.type(input, 'death');

    // After the 250ms debounce window, exactly one new search should
    // fire with the final string — not five (one per keystroke).
    await waitFor(
      () => {
        const fresh = harness.searchCalls.slice(baseline);
        expect(fresh.some((c) => c.query === 'death')).toBe(true);
      },
      { timeout: 1500 },
    );
  });

  it('triggers a search with type_filter when a type pill is clicked', async () => {
    harness.defaultResult = makeResult([
      makeEntry({ event_type: 'location' }),
      makeEntry({ event_type: 'death' }),
    ]);
    render(<LogsPane />);

    await waitFor(() => expect(harness.searchCalls.length).toBeGreaterThan(0));
    const baseline = harness.searchCalls.length;

    const pill = await screen.findByRole('button', { name: /^death$/ });
    fireEvent.click(pill);

    await waitFor(() => {
      const fresh = harness.searchCalls.slice(baseline);
      expect(fresh.some((c) => c.type_filter === 'death')).toBe(true);
    });
  });

  it('appends the next page when "Load more" is clicked, using the smallest loaded id as the cursor', async () => {
    const firstPage = [
      makeEntry({ id: 100, summary: 'first-page-newest' }),
      makeEntry({ id: 99, summary: 'first-page-mid' }),
      makeEntry({ id: 98, summary: 'first-page-oldest' }),
    ];
    const secondPage = [
      makeEntry({ id: 97, summary: 'second-page-newest' }),
      makeEntry({ id: 96, summary: 'second-page-oldest' }),
    ];
    // First search returns has_more=true; the load-more call returns
    // the next page with has_more=false (we've reached the tail).
    harness.searchQueue.push(makeResult(firstPage, { total: 5, has_more: true }));
    harness.searchQueue.push(makeResult(secondPage, { total: 5, has_more: false }));

    render(<LogsPane />);

    const loadMore = await screen.findByRole('button', { name: /Load more/i });
    fireEvent.click(loadMore);

    await waitFor(() => {
      const lastCall = harness.searchCalls[harness.searchCalls.length - 1];
      expect(lastCall?.before_id).toBe(98);
    });

    // Both pages now visible — append, not replace.
    await waitFor(() => {
      expect(screen.getByText('second-page-newest')).toBeInTheDocument();
    });
    expect(screen.getByText('first-page-newest')).toBeInTheDocument();
  });

  it('renders the detail drawer into document.body (portal) when a row is clicked', async () => {
    const entry = makeEntry({ id: 42, event_type: 'death', summary: 'died in space' });
    harness.defaultResult = makeResult([entry]);

    const { container } = render(<LogsPane />);

    const row = await screen.findByText('died in space');
    fireEvent.click(row);

    const dialog = await screen.findByRole('dialog');
    // Portaled: dialog must NOT be a descendant of the rendered
    // container, but it IS a descendant of document.body.
    expect(container.contains(dialog)).toBe(false);
    expect(document.body.contains(dialog)).toBe(true);
  });

  it('dismisses the drawer when Escape is pressed', async () => {
    const entry = makeEntry({ id: 7, event_type: 'death', summary: 'died again' });
    harness.defaultResult = makeResult([entry]);

    render(<LogsPane />);
    const row = await screen.findByText('died again');
    fireEvent.click(row);

    expect(await screen.findByRole('dialog')).toBeInTheDocument();

    fireEvent.keyDown(window, { key: 'Escape' });
    await waitFor(() => {
      expect(screen.queryByRole('dialog')).not.toBeInTheDocument();
    });
  });

  it('dismisses the drawer when the Close button is clicked', async () => {
    const entry = makeEntry({ id: 8, event_type: 'death', summary: 'died once more' });
    harness.defaultResult = makeResult([entry]);

    render(<LogsPane />);
    const row = await screen.findByText('died once more');
    fireEvent.click(row);

    const dialog = await screen.findByRole('dialog');
    const closeBtn = within(dialog).getByRole('button', { name: /^Close$/ });
    fireEvent.click(closeBtn);

    await waitFor(() => {
      expect(screen.queryByRole('dialog')).not.toBeInTheDocument();
    });
  });
});
