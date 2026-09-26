/**
 * Release notes as players read them on the web.
 *
 * The server stores one row per release tag (tray and platform separately,
 * see crates/starstats-server/src/releases.rs). Players do not think in
 * tracks, so /changelog shows one entry per day, pairing whichever tray and
 * platform versions shipped that day, with each line tagged Tray or Web.
 */

import type { components as apiSchema } from 'api-client-ts';

export type Release = apiSchema['schemas']['Release'];

export type NoteKind = 'New' | 'Improved' | 'Fixed';
export interface NoteLine {
  text: string;
  surfaces: string[];
  prs: number[];
  roadmap: string | null;
}
export interface NoteGroup {
  kind: NoteKind;
  lines: NoteLine[];
}

export interface ReleaseDay {
  date: string;
  tray: string | null;
  platform: string | null;
  channel: string;
  groups: NoteGroup[];
}

const KIND_ORDER: NoteKind[] = ['New', 'Improved', 'Fixed'];

function groupsOf(r: Release): NoteGroup[] {
  return Array.isArray(r.notes) ? (r.notes as NoteGroup[]) : [];
}

/**
 * Pair releases by day and channel. Lines with the same text (a roadmap
 * item shipped on both tracks) appear once, with both surfaces.
 */
export function pairByDay(releases: readonly Release[]): ReleaseDay[] {
  const days = new Map<string, ReleaseDay & { seen: Map<string, NoteLine> }>();
  for (const r of releases) {
    const key = `${r.released_on}|${r.channel}`;
    let day = days.get(key);
    if (!day) {
      day = { date: r.released_on, tray: null, platform: null, channel: r.channel, groups: [], seen: new Map() };
      days.set(key, day);
    }
    // Several releases of a track on one day: show the highest version.
    if (r.track === 'tray') day.tray = maxVersion(day.tray, r.version);
    else day.platform = maxVersion(day.platform, r.version);
    for (const g of groupsOf(r)) {
      let group = day.groups.find((x) => x.kind === g.kind);
      if (!group) {
        group = { kind: g.kind, lines: [] };
        day.groups.push(group);
      }
      for (const l of g.lines ?? []) {
        const k = `${g.kind}|${l.text}`;
        const existing = day.seen.get(k);
        if (existing) {
          for (const s of l.surfaces ?? []) if (!existing.surfaces.includes(s)) existing.surfaces.push(s);
          continue;
        }
        const copy = { ...l, surfaces: [...(l.surfaces ?? [])], prs: [...(l.prs ?? [])] };
        day.seen.set(k, copy);
        group.lines.push(copy);
      }
    }
  }
  return [...days.values()]
    .map(({ seen: _seen, ...d }) => ({
      ...d,
      groups: d.groups
        .filter((g) => g.lines.length > 0)
        .sort((a, b) => KIND_ORDER.indexOf(a.kind) - KIND_ORDER.indexOf(b.kind)),
    }))
    .sort((a, b) => b.date.localeCompare(a.date));
}

function maxVersion(a: string | null, b: string): string {
  if (!a) return b;
  const x = a.split('.').map(Number);
  const y = b.split('.').map(Number);
  for (let i = 0; i < 3; i++) if (x[i] !== y[i]) return x[i] > y[i] ? a : b;
  return a;
}
