'use client';

import React, {
  useCallback,
  useEffect,
  useId,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
} from 'react';
import { createPortal } from 'react-dom';
import { rankFuzzy } from '@/lib/fuzzy';

/**
 * Text box with a fuzzy-ranked pick-list underneath: the "Add ship…" and
 * "Add cohort…" controls on the comparison tray.
 *
 * WHY THE LIST IS PORTALED
 * ------------------------
 * It used to be an absolutely-positioned child of the input. On the KB
 * detail page that made it unusable: `.ss-card:hover` lifts the card
 * with a `transform`, which creates a stacking context, so the list's
 * `z-index` could not escape its own card and the NEXT card (the
 * comparison charts) painted over it — precisely while the pointer was
 * on the search box. Inside a widget tile the same list would also be
 * clipped by `.hud-tile__body`'s `overflow-y: auto`. So the list renders
 * into `document.body` with `position: fixed` at coordinates measured
 * from the input, the way `InfoTip` and `EntityLink`'s hover card do, and
 * is re-placed on scroll and resize because `fixed` does not follow its
 * anchor.
 *
 * Accessibility: the input is a `combobox` controlling a `listbox`;
 * arrow keys move the active option (announced via
 * `aria-activedescendant`), Enter picks it, Escape closes and clears, and
 * a click outside closes. The outside-click check has a SECOND containment
 * test for the portaled list, which is no longer inside the input's
 * subtree.
 */
export interface PickerItem {
  key: string;
  label: string;
  /** Small dim caption after the label (a cohort's kind, for instance). */
  hint?: string;
}

export interface SearchPickerProps {
  /** Accessible name of the input. */
  label: string;
  placeholder: string;
  items: readonly PickerItem[];
  onPick: (key: string) => void;
  disabled?: boolean;
  /** List every item while the query is empty. For short lists (cohorts)
   *  where browsing beats typing. Default: nothing until typed. */
  browseWhenEmpty?: boolean;
  /** Maximum options shown. */
  limit?: number;
  /** Input min-width in px. */
  minWidth?: number;
}

/** Gap between the input and the list. */
const GAP = 4;
/** Minimum distance the list keeps from any viewport edge. */
const PAD = 8;
/** Narrowest the list gets, even under a narrow input. */
const MIN_LIST_WIDTH = 220;

interface Pos {
  top: number;
  left: number;
  width: number;
}

export function SearchPicker({
  label,
  placeholder,
  items,
  onPick,
  disabled = false,
  browseWhenEmpty = false,
  limit = 8,
  minWidth = 160,
}: SearchPickerProps) {
  const [query, setQuery] = useState('');
  const [open, setOpen] = useState(false);
  const [active, setActive] = useState(0);
  const [pos, setPos] = useState<Pos | null>(null);
  const listId = useId();
  const wrapRef = useRef<HTMLDivElement>(null);
  const inputRef = useRef<HTMLInputElement>(null);
  const listRef = useRef<HTMLUListElement>(null);

  // Portal target only exists on the client. Gate on a mounted flag rather
  // than `typeof document`, so the server render and the first client
  // render agree and hydration doesn't mismatch.
  const [mounted, setMounted] = useState(false);
  useEffect(() => setMounted(true), []);

  const results = useMemo(() => {
    const q = query.trim();
    if (q) return rankFuzzy(q, items, (i) => i.label, limit);
    return browseWhenEmpty ? items.slice(0, limit) : [];
  }, [query, items, browseWhenEmpty, limit]);

  const showing = open && !disabled && results.length > 0;

  // Keep the active row inside the current result set.
  useEffect(() => {
    setActive((a) => Math.min(a, Math.max(0, results.length - 1)));
  }, [results.length]);

  const place = useCallback(() => {
    const input = inputRef.current;
    const list = listRef.current;
    if (!input || !list) return;
    const r = input.getBoundingClientRect();
    const l = list.getBoundingClientRect();
    const vw = document.documentElement.clientWidth;
    const vh = document.documentElement.clientHeight;

    const width = Math.max(r.width, MIN_LIST_WIDTH);
    // Left-align with the input, then pull back inside the viewport.
    let left = r.left;
    left = Math.min(left, vw - PAD - width);
    left = Math.max(PAD, left);

    // Below the input; flip above when there isn't room.
    let top = r.bottom + GAP;
    if (top + l.height > vh - PAD) top = Math.max(PAD, r.top - l.height - GAP);

    setPos({ top: Math.round(top), left: Math.round(left), width: Math.round(width) });
  }, []);

  // Measure before paint so the list never shows at a stale position.
  useLayoutEffect(() => {
    if (!showing) {
      setPos(null);
      return;
    }
    place();
  }, [showing, results, place]);

  // `position: fixed` does not follow the anchor, so track it while open.
  // Capture phase catches scrolls inside any container, not just the window.
  useEffect(() => {
    if (!showing) return;
    const onMove = () => place();
    window.addEventListener('scroll', onMove, true);
    window.addEventListener('resize', onMove);
    return () => {
      window.removeEventListener('scroll', onMove, true);
      window.removeEventListener('resize', onMove);
    };
  }, [showing, place]);

  // Outside pointer closes. The list is portaled, so it is NOT inside the
  // wrapper's subtree — it needs its own containment check.
  useEffect(() => {
    if (!open) return;
    const onDocPointer = (e: Event) => {
      const t = e.target as Node;
      if (wrapRef.current?.contains(t) || listRef.current?.contains(t)) return;
      setOpen(false);
    };
    document.addEventListener('pointerdown', onDocPointer);
    return () => document.removeEventListener('pointerdown', onDocPointer);
  }, [open]);

  const pick = (key: string) => {
    onPick(key);
    setQuery('');
    setOpen(false);
    setActive(0);
    inputRef.current?.focus();
  };

  const onKeyDown = (e: React.KeyboardEvent<HTMLInputElement>) => {
    switch (e.key) {
      case 'ArrowDown':
        e.preventDefault();
        if (!open) setOpen(true);
        else if (results.length > 0) setActive((a) => (a + 1) % results.length);
        break;
      case 'ArrowUp':
        e.preventDefault();
        if (results.length > 0) setActive((a) => (a - 1 + results.length) % results.length);
        break;
      case 'Enter':
        if (showing && results[active]) {
          e.preventDefault();
          pick(results[active].key);
        }
        break;
      case 'Escape':
        e.preventDefault();
        setOpen(false);
        setQuery('');
        break;
      case 'Tab':
        setOpen(false);
        break;
      default:
        break;
    }
  };

  const optionId = (i: number) => `${listId}-opt-${i}`;

  const list = showing ? (
    <ul
      ref={listRef}
      id={listId}
      role="listbox"
      aria-label={label}
      style={{
        position: 'fixed',
        top: pos?.top ?? 0,
        left: pos?.left ?? 0,
        width: pos?.width ?? MIN_LIST_WIDTH,
        visibility: pos ? 'visible' : 'hidden',
        // Above the tile grid and the sticky chrome, matching `.infotip__pop`.
        zIndex: 200,
        listStyle: 'none',
        margin: 0,
        padding: 4,
        maxHeight: 280,
        overflowY: 'auto',
        background: 'var(--void, var(--bg-elev))',
        border: '1px solid rgba(var(--bR), var(--bG), var(--bB), 0.28)',
        boxShadow: '0 6px 24px rgba(0,0,0,0.35)',
      }}
    >
      {results.map((item, i) => {
        const isActive = i === active;
        return (
          <li
            key={item.key}
            id={optionId(i)}
            role="option"
            aria-selected={isActive}
            // Keep focus in the input: a mousedown on the list must not
            // blur (and so close) the control before the click lands.
            onMouseDown={(e) => e.preventDefault()}
            onMouseEnter={() => setActive(i)}
            onClick={() => pick(item.key)}
            style={{
              display: 'flex',
              alignItems: 'baseline',
              gap: 8,
              color: 'var(--beam, var(--fg))',
              fontSize: 13,
              padding: '6px 8px',
              cursor: 'pointer',
              background: isActive
                ? 'rgba(var(--bR), var(--bG), var(--bB), 0.16)'
                : 'transparent',
            }}
          >
            <span>{item.label}</span>
            {item.hint ? (
              <span style={{ fontSize: 11, color: 'var(--fg-muted)' }}>{item.hint}</span>
            ) : null}
          </li>
        );
      })}
    </ul>
  ) : null;

  return (
    <div ref={wrapRef} style={{ position: 'relative' }}>
      <input
        ref={inputRef}
        type="text"
        role="combobox"
        aria-label={label}
        aria-autocomplete="list"
        aria-expanded={showing}
        aria-controls={showing ? listId : undefined}
        aria-activedescendant={showing && results[active] ? optionId(active) : undefined}
        placeholder={placeholder}
        disabled={disabled}
        value={query}
        onChange={(e) => {
          setQuery(e.target.value);
          setOpen(true);
          setActive(0);
        }}
        onFocus={() => setOpen(true)}
        onKeyDown={onKeyDown}
        autoComplete="off"
        spellCheck={false}
        style={{
          fontSize: 12,
          padding: '6px 12px',
          background: 'transparent',
          color: 'var(--beam, var(--fg))',
          border: '1px dashed var(--border, rgba(255,255,255,.18))',
          minWidth,
        }}
      />
      {list && (mounted ? createPortal(list, document.body) : list)}
    </div>
  );
}
