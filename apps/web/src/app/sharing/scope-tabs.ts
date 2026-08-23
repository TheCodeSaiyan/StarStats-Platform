/**
 * Scope-tab vocabulary for the share editor.
 *
 * Moved out of `page.tsx` by the projection port so `ShareEditor` can import
 * it. Values unchanged.
 */
/** Closed vocabulary mirroring `ALLOWED_SCOPE_TABS` in the Rust
 *  validator. Centralising both lists in the page makes it cheap to
 *  add a new tab — bump both sides and the picker just works. */
export const SCOPE_TAB_OPTIONS: ReadonlyArray<{ value: string; label: string }> = [
  { value: 'location', label: 'Location' },
  { value: 'travel', label: 'Travel' },
  { value: 'combat', label: 'Combat' },
  { value: 'loadout', label: 'Loadout' },
  { value: 'stability', label: 'Stability' },
  { value: 'commerce', label: 'Commerce' },
];
