'use client';

import React from 'react';

/**
 * Chrome-level facts that every projection surface needs and none of them
 * should have to fetch.
 *
 * WHY A CONTEXT AND NOT A PROP. The inbound-share count was carried by the flat
 * `AccountMenu` on every signed-in page, fed by a single fetch in the root
 * layout. In the projection each surface builds its own chrome, so restoring
 * the badge by prop would have meant threading it through a dozen shells and
 * every page that renders one — and a badge that appears on some pages and not
 * others is worse than one that appears nowhere, because it teaches the reader
 * the wrong thing about where notifications live.
 *
 * `layout.tsx` still wraps every route, still has the count, and this is the
 * one place that survived the port unchanged. So the value goes in here and
 * `PaneSurface` reads it.
 *
 * Defaults to zero, so a surface rendered outside the provider (a test, a
 * boundary) is silent rather than broken.
 */
export interface ShellData {
  /** Records other people have shared with this reader, unexpired. */
  inboundShares: number;
}

const Ctx = React.createContext<ShellData>({ inboundShares: 0 });

export function ShellDataProvider({
  inboundShares,
  children,
}: ShellData & { children: React.ReactNode }) {
  const value = React.useMemo(() => ({ inboundShares }), [inboundShares]);
  return <Ctx.Provider value={value}>{children}</Ctx.Provider>;
}

export function useShellData(): ShellData {
  return React.useContext(Ctx);
}
