import { describe, it, expect } from 'vitest';
import fs from 'node:fs';
import path from 'node:path';
import { SHARE_SCOPES, splitShareScopes } from './share-scopes';
import { DEFAULT_SHARE_SCOPES } from '@/app/_components/widgets/types';

describe('share scopes', () => {
  it('covers every scope the API defines, exactly once', () => {
    // `DEFAULT_SHARE_SCOPES` is typed as the full `WidgetShareScopesApi`, so
    // its keys ARE the server's vocabulary. A scope the server has and this
    // list does not is a switch a pilot can set and no screen will ever name.
    const apiKeys = Object.keys(DEFAULT_SHARE_SCOPES).sort();
    const listed = SHARE_SCOPES.map((s) => s.key).sort();
    expect(listed).toEqual(apiKeys);
  });

  it('accounts for every scope as published or withheld', () => {
    // Both halves are shown on the public profile, and the point of showing
    // both is that no scope falls through the gap: one that appears in
    // neither list reads as absent data rather than as a withheld choice.
    const mixed = {
      ...DEFAULT_SHARE_SCOPES,
      travel: true,
      economy: true,
    };
    const { published, withheld } = splitShareScopes(mixed);
    expect(published).toEqual(['Economy', 'Travel']);
    expect(published.length + withheld.length).toBe(SHARE_SCOPES.length);
    expect(withheld).toContain('Records');
  });

  it('reports all-false as five withheld, never as an empty statement', () => {
    const { published, withheld } = splitShareScopes(DEFAULT_SHARE_SCOPES);
    expect(published).toEqual([]);
    expect(withheld.length).toBe(SHARE_SCOPES.length);
  });

  it('is the only scope-label list', () => {
    // The `/me` catalogue records what a second copy costs: "This file
    // duplicated it at first and immediately drifted." These labels started
    // inside `/settings/widget-sharing` as a local `WIDGET_LABELS`, which is
    // why the public profile could not name a scope without copying them.
    //
    // Checked by IMPORT, not by scanning for the strings. Two widgets happen
    // to title themselves "Combat & Missions" as well — that is the widget's
    // name, a different vocabulary that coincides, and failing on it would be
    // the test asserting a relationship the product does not have.
    const settings = fs.readFileSync(
      path.join(process.cwd(), 'src/app/settings/widget-sharing/page.tsx'),
      'utf8',
    );
    expect(settings).toContain("from '@/lib/share-scopes'");
    expect(settings).not.toMatch(/label:\s*'Combat & Missions'/);
  });
});
