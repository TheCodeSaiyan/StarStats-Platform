'use client';

import React from 'react';

/**
 * The customization system.
 *
 * Every removable thing in a projection is an *element*: a callout, a lens, a
 * plane, a trace, a pane section. `useLayout` owns which ones are enabled and
 * in what order; `LayoutEditor` is the panel that adds, removes and reorders
 * them. The catalogue is DATA, not markup — a screen declares what CAN be
 * shown, the layout decides what IS shown, so adding a new element is one entry.
 *
 * NAMING: this is deliberately "layout", never "loadout". In Star Citizen a
 * loadout is the gear you spawned with, and the product has a real page for it
 * (`/me/loadout`). Using the same word for element arrangement would collide
 * with the one meaning every player already has.
 *
 * PERSISTENCE (port change). The kit persisted to `localStorage`, which its own
 * notes flag as a stand-in: "the product stores the tile layout ON THE ACCOUNT
 * — the guide is explicit that it follows you to another browser." So this hook
 * takes an injected `persist` callback instead of touching storage itself, and
 * the app hands it a server action. There is deliberately no localStorage
 * fallback: a silent per-device layout would be a regression against a product
 * behaviour that already works.
 */
export interface CatalogueEntry {
  /** PERSISTED. Renaming an id silently drops a reader's saved layout. */
  id: string;
  name: string;
  /** Editor grouping — "Callouts", "Lenses", "Lens panes". */
  group?: string;
  /** Off by default when explicitly false. */
  on?: boolean;
  /** Tooltip clarifying what the element is, shown on the name. */
  hint?: string;
}

/**
 * What the last write is doing, so the editor can say so.
 *
 * `pending` also carries the ids ADDED since the server last rendered. Those
 * cannot appear yet — the elements are built server-side from the saved
 * layout, so an id the server has not seen has no data and no view — and the
 * editor was reporting them as projected regardless. Measured: adding a
 * widget moved the counter from "1 of 22 projected" to "2 of 22" while the
 * number of drawn planes stayed at zero.
 */
export type LayoutSaveState =
  | { kind: 'idle' }
  | { kind: 'saving' }
  | { kind: 'saved' }
  | { kind: 'error' };

export interface LayoutApi {
  ids: string[];
  has: (id: string) => boolean;
  add: (id: string) => void;
  remove: (id: string) => void;
  toggle: (id: string) => void;
  /**
   * Step `id` one position through `peers` — the ids as the READER SEES THEM.
   *
   * It used to swap neighbours in the global `ids` array, which the editor
   * does not render: rows are drawn per group in catalogue order, so a reorder
   * moved the data and nothing on screen. Measured: moving "Top routes"
   * earlier wrote `[routes, travel, spend]` while the visible list stayed
   * `[Spending, Quantum transits, Top routes]`. Passing the displayed sequence
   * makes the move land where the reader is looking; the global order is kept
   * coherent by splicing `id` next to the peer it stepped over.
   */
  move: (id: string, dir: number, peers?: readonly string[]) => void;
  reset: () => void;
  /** State of the most recent write. */
  save: LayoutSaveState;
  /** Ids added since the server last built the elements — enabled in the
   *  layout but not yet drawable. Empty once a refresh has landed. */
  awaitingRender: string[];
  /** Enabled elements in layout order — map this to render. */
  projected: CatalogueEntry[];
}

export interface UseLayoutOptions {
  /** Ids to start from — the reader's saved layout, fetched server-side. */
  initial?: string[];
  /**
   * Called with the full id list after every change. Wire this to the server
   * action that writes `PUT /v1/users/me/profile-layout`.
   *
   * The UI still moves immediately — a reader mid-edit must not be blocked on
   * a round trip — but the promise IS awaited so the editor can say whether
   * the write landed. It used to be fire-and-forget, which meant a failed
   * save looked exactly like a successful one.
   */
  persist?: (ids: string[]) => void | Promise<void>;
}

export function useLayout(
  surface: string,
  catalogue: readonly CatalogueEntry[],
  options: UseLayoutOptions = {},
): LayoutApi {
  const defaults = React.useMemo(
    () => catalogue.filter((e) => e.on !== false).map((e) => e.id),
    [catalogue],
  );
  const initial = options.initial ?? defaults;
  const [ids, setIds] = React.useState<string[]>(initial);
  const { persist } = options;

  // A saved layout can name an element that no longer exists (renamed or
  // removed). Drop unknown ids rather than rendering a hole.
  const known = React.useMemo(
    () => new Set(catalogue.map((e) => e.id)),
    [catalogue],
  );

  const [save, setSave] = React.useState<LayoutSaveState>({ kind: 'idle' });
  /**
   * The ids the SERVER knew about when it built the elements.
   *
   * Anything added after that cannot be drawn — its data was never fetched —
   * so the editor must not claim it is projected. `initial` changes when a
   * refresh lands with the new layout, which is what clears the pending set.
   */
  const serverIds = React.useMemo(() => new Set(initial), [initial]);

  /**
   * ADOPT WHAT THE SERVER LAST SENT.
   *
   * `ids` seeds from `initial` once and never looked at it again, so a write
   * the server REFUSED left the editor showing the reader's change for the
   * rest of the session — ticked in the list, absent from the stage, and
   * gone on the next full load. After a successful save the refresh echoes
   * back what we sent and this is a no-op; after a failed one it snaps to the
   * truth. Keyed on the contents, so a fresh array of the same ids does not
   * clobber an edit in flight.
   */
  const initialKey = initial.join('|');
  const adopted = React.useRef(initialKey);
  React.useEffect(() => {
    if (adopted.current === initialKey) return;
    adopted.current = initialKey;
    setIds(initial.filter((id) => known.has(id)));
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [initialKey]);

  const commit = React.useCallback(
    (next: string[]) => {
      const clean = next.filter((id) => known.has(id));
      // Optimistic: the reader's own click must not wait on a round trip.
      setIds(clean);
      if (!persist) return;
      let result: void | Promise<void>;
      try {
        result = persist(clean);
      } catch {
        setSave({ kind: 'error' });
        return;
      }
      if (!result || typeof (result as Promise<void>).then !== 'function') {
        // A synchronous persist tells us nothing about the write; say nothing.
        return;
      }
      setSave({ kind: 'saving' });
      (result as Promise<void>).then(
        () => setSave({ kind: 'saved' }),
        // A failed write used to look exactly like a successful one.
        () => setSave({ kind: 'error' }),
      );
    },
    [known, persist],
  );

  return {
    ids,
    save,
    awaitingRender: ids.filter((id) => !serverIds.has(id)),
    has: (id: string) => ids.includes(id),
    add: (id: string) => {
      if (!ids.includes(id)) commit([...ids, id]);
    },
    remove: (id: string) => commit(ids.filter((x) => x !== id)),
    toggle: (id: string) =>
      ids.includes(id)
        ? commit(ids.filter((x) => x !== id))
        : commit([...ids, id]),
    move: (id: string, dir: number, peers?: readonly string[]) => {
      // Step through what is on screen, fall back to the whole layout.
      const seq = (peers ?? ids).filter((x) => ids.includes(x));
      const i = seq.indexOf(id);
      const j = i + dir;
      if (i < 0 || j < 0 || j >= seq.length) return;
      const over = seq[j];
      const next = ids.filter((x) => x !== id);
      const at = next.indexOf(over);
      if (at < 0) return;
      next.splice(dir < 0 ? at : at + 1, 0, id);
      commit(next);
    },
    reset: () => commit(defaults),
    projected: ids
      .map((id) => catalogue.find((e) => e.id === id))
      .filter((e): e is CatalogueEntry => Boolean(e)),
  };
}

export interface LayoutEditorProps {
  catalogue?: readonly CatalogueEntry[];
  layout: LayoutApi;
  onClose?: () => void;
  /** Docks into page flow (the Calibrate screen) instead of floating. */
  docked?: boolean;
  title?: string;
}

export function LayoutEditor({
  catalogue = [],
  layout,
  onClose,
  docked = false,
  title = 'Projection layout',
}: LayoutEditorProps) {
  /**
   * ROWS ARE DRAWN IN LAYOUT ORDER, NOT CATALOGUE ORDER.
   *
   * The list was always catalogue order, so the reorder controls moved an
   * array the reader could not see — and order is not cosmetic here: the
   * callout field draws the first six and reports the rest as undrawn, so the
   * order IS the choice of which six appear. Enabled elements now sit at the
   * top of their group in the order they will be used; the ones that are off
   * have no position yet and keep catalogue order below them.
   */
  const pos = new Map(layout.ids.map((id, i) => [id, i] as const));
  const groups: { g: string; items: CatalogueEntry[] }[] = [];
  catalogue.forEach((e) => {
    const g = e.group || 'Elements';
    let bucket = groups.find((x) => x.g === g);
    if (!bucket) {
      bucket = { g, items: [] };
      groups.push(bucket);
    }
    bucket.items.push(e);
  });
  groups.forEach((grp) => {
    grp.items = grp.items
      .map((e, i) => ({ e, i }))
      .sort((a, b) => {
        const pa = pos.get(a.e.id);
        const pb = pos.get(b.e.id);
        if (pa != null && pb != null) return pa - pb;
        // Enabled first: an element that is off has no place in the order.
        if (pa != null) return -1;
        if (pb != null) return 1;
        return a.i - b.i;
      })
      .map((x) => x.e);
  });
  const count = layout.ids.length;

  return (
    <div
      className={docked ? 'hp-layout hp-layout--docked' : 'hp-layout'}
      role={docked ? 'group' : 'dialog'}
      aria-label={title}
    >
      <h3>{title}</h3>
      {/* THE COUNT USED TO LIE, TWICE OVER.
          It read "N of M projected" the instant an id was added, but elements
          are built server-side from the saved layout — one the server has not
          seen draws nothing. Measured: the counter moved 1 -> 2 while the
          drawn planes stayed at 0. That part is handled by holding the id in
          `awaitingRender` until a refresh lands.
          The second lie was the WORD. Enabled is not projected: the callout
          field draws six, the rest are reported undrawn, and lens panes only
          show their own lens. A reader with everything switched on was told
          "23 of 23 projected" while six were on the ring. It now counts what
          it can actually prove — how many are switched on. */}
      <div className="note" role="status" aria-live="polite">
        {count - layout.awaitingRender.length} of {catalogue.length} on
        {layout.awaitingRender.length > 0 ? (
          <>
            {' · '}
            <b className="pend">
              {layout.awaitingRender.length} loading
            </b>
          </>
        ) : null}
        {' · '}
        {layout.save.kind === 'saving'
          ? 'saving…'
          : layout.save.kind === 'error'
            ? <b className="err">could not save — your change may not stick</b>
            : layout.save.kind === 'saved'
              ? 'saved to your account'
              : 'saved to your account'}
      </div>
      {groups.map((grp) => {
        // The sequence the arrows step through: this group's enabled ids in
        // the order they are drawn. Ends are disabled, so a press that cannot
        // move anything is never offered.
        const ordered = grp.items
          .filter((x) => layout.has(x.id))
          .map((x) => x.id);
        return (
        <div key={grp.g}>
          <div className="grp">{grp.g}</div>
          {grp.items.map((e) => {
            const on = layout.has(e.id);
            return (
              <div
                className="hp-el"
                key={e.id}
                data-on={on ? 'true' : 'false'}
                // The row a reader just added, before the server has rebuilt.
                data-pending={
                  layout.awaitingRender.includes(e.id) ? 'true' : undefined
                }
              >
                <span className="gp" aria-hidden="true">
                  {on ? '⠿' : '·'}
                </span>
                <span className="nm" title={e.hint || e.name}>
                  {e.name}
                </span>
                {/* Both directions. There was only an "earlier" control, so
                    an element could be moved up and never back down — the
                    only way to demote one was to promote everything else. */}
                {on ? (
                  <button
                    type="button"
                    aria-label={`Move ${e.name} earlier`}
                    disabled={ordered.indexOf(e.id) <= 0}
                    onClick={() => layout.move(e.id, -1, ordered)}
                  >
                    ↑
                  </button>
                ) : (
                  <span />
                )}
                {on ? (
                  <button
                    type="button"
                    aria-label={`Move ${e.name} later`}
                    disabled={ordered.indexOf(e.id) >= ordered.length - 1}
                    onClick={() => layout.move(e.id, 1, ordered)}
                  >
                    ↓
                  </button>
                ) : (
                  <span />
                )}
                {on ? (
                  <button
                    type="button"
                    className="del"
                    aria-label={`Remove ${e.name}`}
                    onClick={() => layout.remove(e.id)}
                  >
                    −
                  </button>
                ) : (
                  <button
                    type="button"
                    className="add"
                    aria-label={`Add ${e.name}`}
                    onClick={() => layout.add(e.id)}
                  >
                    +
                  </button>
                )}
              </div>
            );
          })}
        </div>
        );
      })}
      <div className="ft">
        <button
          type="button"
          className="hp-btn hp-btn--ghost"
          onClick={layout.reset}
        >
          Reset
        </button>
        <button
          type="button"
          className="hp-btn hp-btn--primary"
          onClick={onClose}
        >
          Done
        </button>
      </div>
    </div>
  );
}

/**
 * Capitalised alias of `useLayout`. The design-system bundle only exposes
 * exports that start with a capital letter, so this is the name consumers get
 * off the namespace. Identical function.
 */
export const UseLayout = useLayout;
