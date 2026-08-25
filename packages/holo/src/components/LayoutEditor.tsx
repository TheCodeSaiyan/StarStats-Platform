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
  move: (id: string, dir: number) => void;
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
    move: (id: string, dir: number) => {
      const i = ids.indexOf(id);
      const j = i + dir;
      if (i < 0 || j < 0 || j >= ids.length) return;
      const next = [...ids];
      next[i] = next[j];
      next[j] = id;
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
  const count = layout.ids.length;

  return (
    <div
      className={docked ? 'hp-layout hp-layout--docked' : 'hp-layout'}
      role={docked ? 'group' : 'dialog'}
      aria-label={title}
    >
      <h3>{title}</h3>
      {/* THE COUNT USED TO LIE. It read "N of M projected" the instant an id
          was added, but the elements are built server-side from the saved
          layout — a widget the server has not seen has no data and draws
          nothing. Measured: the counter moved 1 -> 2 while the drawn planes
          stayed at 0. Now the line separates what IS projected from what is
          waiting on a refresh, and says whether the write actually landed. */}
      <div className="note" role="status" aria-live="polite">
        {count - layout.awaitingRender.length} of {catalogue.length} projected
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
      {groups.map((grp) => (
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
                {on ? (
                  <button
                    type="button"
                    aria-label={`Move ${e.name} earlier`}
                    onClick={() => layout.move(e.id, -1)}
                  >
                    ↑
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
      ))}
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
