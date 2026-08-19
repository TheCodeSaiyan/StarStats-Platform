'use client';

import React, {
  createContext,
  useCallback,
  useContext,
  useState,
} from 'react';

/**
 * Edit mode is CLIENT state, not a URL param.
 *
 * It used to live in `?edit=1` (via `router.replace`), which in the Next
 * App Router triggers a SERVER round-trip — and `/me` renders the widget
 * canvas server-side, re-fetching every widget's data. So just clicking
 * "Edit" (or "Done") stalled 10–30s while 19 widgets re-fetched. Edit mode
 * is a pure presentational toggle with zero data dependency, so it belongs
 * in React state shared via context between the `EditToggle` (in the
 * control strip) and the `SortableProfileWidgets` grid.
 *
 * The two live in different subtrees of the server page, so a provider
 * wraps both (see `me/page.tsx` / `u/[handle]/page.tsx`). Without a
 * provider the hook falls back to local component state so isolated
 * renders (Storybook, unit tests that don't mock this module) still work.
 */
interface EditModeValue {
  isEditing: boolean;
  setEditing: (next: boolean) => void;
}

const EditModeContext = createContext<EditModeValue | null>(null);

export function EditModeProvider({ children }: { children: React.ReactNode }) {
  const [isEditing, setIsEditing] = useState(false);
  const setEditing = useCallback((next: boolean) => setIsEditing(next), []);
  return React.createElement(
    EditModeContext.Provider,
    { value: { isEditing, setEditing } },
    children,
  );
}

export function useEditMode(): EditModeValue {
  const ctx = useContext(EditModeContext);
  // Hooks must run unconditionally; the local fallback is only used when
  // there's no provider above (never in the real dashboard).
  const [localEditing, setLocalEditing] = useState(false);
  const localSet = useCallback((next: boolean) => setLocalEditing(next), []);
  return ctx ?? { isEditing: localEditing, setEditing: localSet };
}
