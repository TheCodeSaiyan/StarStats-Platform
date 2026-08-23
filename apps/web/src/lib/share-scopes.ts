import type { WidgetShareScopesApi } from '@/lib/api';

/**
 * The per-scope sharing vocabulary — one list, two readers.
 *
 * These five booleans are a pilot's actual privacy switches: what
 * `/settings/widget-sharing` writes, and what `GET /v1/public/{handle}/share-scopes`
 * hands to anyone who asks. The labels lived inside the settings page as a
 * local `WIDGET_LABELS`, so the public profile — the one screen where the
 * question "what does this pilot publish?" is actually asked — had no way to
 * name them without a second copy.
 *
 * The `/me` catalogue records what happens to a second copy: "This file
 * duplicated it at first and immediately drifted." One list, and it is this
 * one.
 *
 * NOT THE SAME THING AS A LENS. `lib/lens.ts` has six lenses (activity,
 * travel, combat, loadout, commerce, plus All) and these are five scopes; they
 * overlap but do not correspond, and mapping one onto the other would invent a
 * relationship the backend does not have. A page that wants to say what is
 * published says it in THIS vocabulary, because it is the vocabulary the pilot
 * actually set.
 */
export interface ShareScopeMeta {
  key: keyof WidgetShareScopesApi;
  label: string;
  description: string;
}

export const SHARE_SCOPES: readonly ShareScopeMeta[] = [
  {
    key: 'combat_mission',
    label: 'Combat & Missions',
    description: 'Player deaths, vehicle losses, and mission start/end counts.',
  },
  {
    key: 'economy',
    label: 'Economy',
    description: 'Shop purchases and commodity trade counts.',
  },
  {
    key: 'travel',
    label: 'Travel',
    description: 'Quantum jumps, server hops, and other movement events.',
  },
  {
    key: 'records',
    label: 'Records',
    description:
      'Longest session, busiest session, biggest trade, and survival streak.',
  },
  {
    key: 'recent_activity',
    label: 'Recent activity',
    description: 'A live feed of your most recent logged events.',
  },
];

/**
 * Split the scopes into what this pilot publishes and what they withhold.
 *
 * Both halves are returned because both are shown. A public profile that
 * listed only the published set would let a sparse page read as a quiet pilot
 * rather than a private one — `Profile.jsx` is explicit that it "must never
 * imply data it is not allowed to show".
 */
export function splitShareScopes(scopes: WidgetShareScopesApi): {
  published: string[];
  withheld: string[];
} {
  const published: string[] = [];
  const withheld: string[] = [];
  for (const s of SHARE_SCOPES) {
    (scopes[s.key] ? published : withheld).push(s.label);
  }
  return { published, withheld };
}
