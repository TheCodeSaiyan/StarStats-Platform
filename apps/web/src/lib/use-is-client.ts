import { useSyncExternalStore } from 'react';

const subscribe = () => () => {};
const clientSnapshot = () => true;
const serverSnapshot = () => false;

/**
 * `false` for the server render and the hydration render, `true` from the
 * first client render after that — the same thing the old
 * `useState(false)` + `useEffect(() => setMounted(true), [])` gate gave,
 * without the effect. React's own answer for "is this the client": the
 * server snapshot is what hydration compares against, so markup matches,
 * and the client snapshot takes over in the next render without a state
 * write inside an effect (`react-hooks/set-state-in-effect`).
 *
 * Use it to gate anything that needs `document` or `navigator` — a portal
 * target, a browser-only default — and read the browser value at render
 * time behind it rather than copying it into state.
 */
export function useIsClient(): boolean {
  return useSyncExternalStore(subscribe, clientSnapshot, serverSnapshot);
}
