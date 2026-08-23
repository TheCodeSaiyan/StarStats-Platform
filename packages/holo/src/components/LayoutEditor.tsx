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

export interface LayoutApi {
  ids: string[];
  has: (id: string) => boolean;
  add: (id: string) => void;
  remove: (id: string) => void;
  toggle: (id: string) => void;
  move: (id: string, dir: number) => void;
  reset: () => void;
  /** Enabled elements in layout order — map this to render. */
  projected: CatalogueEntry[];
}

export interface UseLayoutOptions {
  /** Ids to start from — the reader's saved layout, fetched server-side. */
  initial?: string[];
  /**
   * Called with the full id list after every change. Wire this to the server
   * action that writes `PUT /v1/users/me/profile-layout`. Persistence is
   * fire-and-forget: the UI has already moved, and a failed write must not
   * strand the reader mid-edit.
   */
  persist?: (ids: string[]) => void;
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

  const commit = React.useCallback(
    (next: string[]) => {
      const clean = next.filter((id) => known.has(id));
      setIds(clean);
      persist?.(clean);
    },
    [known, persist],
  );

  return {
    ids,
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
      <div className="note">
        {count} of {catalogue.length} projected · saved to your account
      </div>
      {groups.map((grp) => (
        <div key={grp.g}>
          <div className="grp">{grp.g}</div>
          {grp.items.map((e) => {
            const on = layout.has(e.id);
            return (
              <div className="hp-el" key={e.id} data-on={on ? 'true' : 'false'}>
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
