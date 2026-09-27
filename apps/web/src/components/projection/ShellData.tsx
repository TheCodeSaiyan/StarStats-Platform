'use client';

import React from 'react';
import type { NavSection } from 'holo';

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
  /**
   * Unread notifications (friend requests, accepted requests). Same reason
   * as the share count for living here: a badge that shows on some pages and
   * not others teaches the reader the wrong place to look.
   */
  unreadNotifications: number;
  /** Unread staff news plus unread shipped roadmap items: What's New. */
  unreadWhatsNew: number;
  /** Players waiting on the reader's open Looking for Group post. */
  crewPending: number;
}

const Ctx = React.createContext<ShellData>({
  inboundShares: 0,
  unreadNotifications: 0,
  unreadWhatsNew: 0,
  crewPending: 0,
});

export function ShellDataProvider({
  inboundShares,
  unreadNotifications,
  unreadWhatsNew,
  crewPending,
  children,
}: ShellData & { children: React.ReactNode }) {
  const value = React.useMemo(
    () => ({ inboundShares, unreadNotifications, unreadWhatsNew, crewPending }),
    [inboundShares, unreadNotifications, unreadWhatsNew, crewPending],
  );
  return <Ctx.Provider value={value}>{children}</Ctx.Provider>;
}

export function useShellData(): ShellData {
  return React.useContext(Ctx);
}

/**
 * The site nav with its counts attached: players waiting on your group, on
 * Crew. Every shell that draws the bar runs its nav through this, for the
 * same reason the account badges live in this context: a count that shows on
 * some pages and not others teaches the reader the wrong place to look.
 */
export function withNavBadges(nav: NavSection[], data: ShellData): NavSection[] {
  if (data.crewPending <= 0) return nav;
  return nav.map((sec) => ({
    ...sec,
    items: sec.items.map((it) => (it.id === 'lfg' ? { ...it, badge: data.crewPending } : it)),
  }));
}

/** {@link withNavBadges} from the shell's own data. */
export function useNavWithBadges(nav: NavSection[]): NavSection[] {
  const data = useShellData();
  return React.useMemo(() => withNavBadges(nav, data), [nav, data]);
}
