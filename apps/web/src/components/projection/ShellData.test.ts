import { describe, expect, it } from 'vitest';
import type { NavSection } from 'holo';
import { withNavBadges, type ShellData } from './ShellData';

const nav: NavSection[] = [
  {
    title: 'You',
    items: [
      { id: 'me', label: 'Projection', href: '/me' },
      { id: 'lfg', label: 'Crew', href: '/lfg' },
    ],
  },
];

const data = (crewPending: number): ShellData => ({
  inboundShares: 0,
  unreadNotifications: 0,
  unreadWhatsNew: 0,
  crewPending,
});

describe('withNavBadges', () => {
  it('puts the waiting count on Crew and nowhere else', () => {
    const [sec] = withNavBadges(nav, data(2));
    expect(sec.items.find((i) => i.id === 'lfg')?.badge).toBe(2);
    expect(sec.items.find((i) => i.id === 'me')?.badge).toBeUndefined();
  });

  it('leaves the nav untouched when nobody is waiting', () => {
    expect(withNavBadges(nav, data(0))).toBe(nav);
  });
});
