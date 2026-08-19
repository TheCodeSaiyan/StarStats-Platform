import React from 'react';
import type { Route } from 'next';
import { getPublicRsiOrgs, type RsiOrgsSnapshot } from '@/lib/api';
import { logger } from '@/lib/logger';
import { defineWidget } from './kit/defineWidget';
import { RankedList } from './kit/archetypes';
import { fmtRelative } from './kit/format';

/**
 * `orgs` — the citizen's public RSI org memberships (`GET
 * /v1/public/u/{handle}/orgs`, public-or-shared, server-gated).
 *
 * Migrated to the kit: it used to delegate to `<OrgsCardInner>`, whose
 * page-styled `h2`s (fontSize 17) rendered oversized in a widget tile and
 * inflated the content-auto-fit measurement (the 77px waste). Now a compact
 * `RankedList` (org name → rank), which measures cleanly and matches every
 * other tile. `OrgsCardInner` stays in use for the standalone /orgs surface.
 */
interface OrgsData {
  orgs: RsiOrgsSnapshot['orgs'];
  capturedAt: string;
}

export const orgsWidget = defineWidget<OrgsData>({
  id: 'orgs',
  eyebrow: 'Affiliations',
  // Public data; the server enforces public-or-shared visibility. Empty/404
  // (no snapshot / not public / not shared) → load returns null → no tile.
  visibility: 'public',
  async load(ctx) {
    let snapshot: RsiOrgsSnapshot | null = null;
    try {
      snapshot = await getPublicRsiOrgs(ctx.ownerHandle);
    } catch (err) {
      logger.warn(
        { err, call: 'widget.orgs', handle: ctx.ownerHandle },
        'orgs fetch failed',
      );
      return null;
    }
    if (!snapshot || snapshot.orgs.length === 0) return null;
    return { orgs: snapshot.orgs, capturedAt: snapshot.captured_at };
  },
  body(data) {
    // Main org first, then alphabetical — stable across reloads.
    const sorted = [...data.orgs].sort((a, b) => {
      if (a.is_main !== b.is_main) return a.is_main ? -1 : 1;
      return a.name.localeCompare(b.name);
    });
    const rows = sorted.map((o) => ({
      key: o.sid,
      label: o.is_main ? `${o.name} ★` : o.name,
      value: o.rank ?? o.sid,
    }));
    return (
      <RankedList
        rows={rows}
        cap={8}
        seeMore={{ href: '/orgs' as Route, label: () => 'All orgs →' }}
        note={`Snapshot ${fmtRelative(data.capturedAt, Date.now())}`}
      />
    );
  },
});
