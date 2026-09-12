import { test, expect } from '@playwright/test';
import { loginAs, resetScenario, scenarioFor, setScenario } from './helpers/api-mock';

/**
 * A WIDGET TILE NEVER SCROLLS.
 *
 * `grid-layout.ts` records that the per-widget row counts "are chosen to FIT
 * each widget's bounded summary content EXACTLY". Zero slack, by design — so a
 * tile whose content is taller than its body has silently started hiding part
 * of itself behind `overflow-y: auto`.
 *
 * That slack was spent once already: raising the micro tier 8.5px -> 10px for
 * legibility pushed a compact travel tile to 52px of body against 57px of
 * content, and the body collapsed to clientHeight 0 — content and the InfoTip
 * control inside it in the DOM and clipped out of existence. Both fixes for
 * that were verified by hand on a few tiles. This is the same check over a
 * full layout, which is what the overnight review asked for.
 *
 * It found four on its first run, stable from 500ms to 6s (so not a settling
 * race), including a body still collapsing to zero months after the guard for
 * it was believed done — the guard had lost on specificity to the free-grid's
 * own `min-height: 0`.
 */
const MIN_TILES = 4;

/**
 * Tiles whose content does not fit their tuned row count at the current type
 * scale. Each is named with its measured overflow rather than the check being
 * relaxed, because closing them means changing per-widget row counts — and
 * those are persisted in every reader's saved layout, so re-tuning silently
 * moves everyone's tiles. That is a product decision, not a test fix.
 *
 * The recorded number is a CEILING: a listed tile that overflows by more than
 * this still fails. An allowlist that absorbs growth is just a slower way of
 * having no test.
 */
const KNOWN_TIGHT: Record<string, number> = {
  // The only one left after the row model was corrected, and it is the one
  // tile whose chrome is not the usual 59px: its header carries an extra
  // line, measured at 71px. `TILE_CHROME_PX` is a single figure for every
  // widget, so it under-provisions this one by ~12px — a third of a row.
  // Raising the constant to suit it would add a wasted row to every other
  // tile, which is the opposite failure.
  entities: 26, // 47px of content in a 21px body
};

test('no widget tile hides its own content', async ({ page, request }) => {
  test.setTimeout(180_000);
  await resetScenario(request);
  await setScenario(request, scenarioFor('tile-fit'));
  await loginAs(page, { handle: 'TestPilot' });

  await page.goto('/u/TestPilot', { waitUntil: 'domcontentloaded', timeout: 60_000 });
  await page.waitForLoadState('networkidle').catch(() => {});
  await page.waitForTimeout(1500);

  const result = await page.evaluate(() => {
    const bodies = Array.from(
      document.querySelectorAll<HTMLElement>('.hud-tile__body, .metric-card__body'),
    );
    return {
      count: bodies.length,
      tiles: bodies.map((el) => ({
        id: el.closest<HTMLElement>('[data-widget-id]')?.dataset.widgetId ?? '?',
        client: el.clientHeight,
        scroll: el.scrollHeight,
      })),
    };
  });

  // Guard against a vacuous pass: a page that rendered no tiles satisfies "no
  // tile overflows" perfectly, which is how a green gate proves nothing.
  expect(
    result.count,
    `expected at least ${MIN_TILES} tiles to measure, saw ${result.count} — the ` +
      'fixtures or layout are not producing a populated dashboard, so this run ' +
      'measured nothing',
  ).toBeGreaterThanOrEqual(MIN_TILES);

  const failures: string[] = [];
  for (const t of result.tiles) {
    if (t.client === 0) {
      failures.push(`${t.id}: body collapsed to 0 with ${t.scroll}px of content`);
      continue;
    }
    const over = t.scroll - t.client;
    if (over <= 1) continue;
    const allowed = KNOWN_TIGHT[t.id];
    if (allowed === undefined) {
      failures.push(`${t.id}: ${t.scroll}px of content in a ${t.client}px body`);
    } else if (over > allowed) {
      failures.push(
        `${t.id}: overflow grew to ${over}px (was ${allowed}px) — a known-tight ` +
          'tile got tighter, which the allowlist must not absorb',
      );
    }
  }
  expect(failures, failures.join('\n')).toEqual([]);
});

test('no widget tile header spills out of its tile', async ({ page, request }) => {
  /**
   * The vertical sibling of the check above, for the axis a narrow tile
   * actually loses.
   *
   * `.hud-tile__hd` is a `flex-wrap: nowrap` row carrying an eyebrow and a
   * title. In a full-width tile they fit; in a 6-column one they do not, and
   * `nowrap` has nowhere to put the overflow — so the title runs past the tile
   * edge and `.hud-tile { overflow: hidden }` cuts it. Measured on the public
   * profile's default layout at 1440x900: the Affiliations tile ends at x=487
   * and its "Orgs" title runs to x=501, and the Cross-session rollups tile
   * loses the second line of its title entirely.
   *
   * A clipped label is not a visibility failure — every one of these elements
   * reports `visible`, and the tile-fit check above only measures height — so
   * the assertion is the geometry: no header child may end past its tile's
   * content edge.
   */
  test.setTimeout(180_000);
  await resetScenario(request);
  await setScenario(request, scenarioFor('tile-fit'));
  await loginAs(page, { handle: 'TestPilot' });

  await page.goto('/u/TestPilot', { waitUntil: 'domcontentloaded', timeout: 60_000 });
  await page.waitForLoadState('networkidle').catch(() => {});
  await page.waitForTimeout(1500);

  const spills = await page.evaluate(() => {
    const tiles = Array.from(document.querySelectorAll<HTMLElement>('.hud-tile'));
    const out: { id: string; label: string; by: number }[] = [];
    for (const tile of tiles) {
      const tb = tile.getBoundingClientRect();
      const padRight = parseFloat(getComputedStyle(tile).paddingRight) || 0;
      const limit = tb.right - padRight;
      const hd = tile.querySelector<HTMLElement>('.hud-tile__hd');
      if (!hd) continue;
      for (const child of Array.from(hd.children)) {
        const cb = child.getBoundingClientRect();
        if (cb.width === 0) continue;
        const by = Math.round(cb.right - limit);
        if (by > 1) {
          out.push({
            id: tile.closest<HTMLElement>('[data-widget-id]')?.dataset.widgetId ?? '?',
            label: (child.textContent ?? '').trim().slice(0, 24),
            by,
          });
        }
      }
    }
    return { count: tiles.length, out };
  });

  // Same vacuous-pass guard as above: no tiles is a perfect score.
  expect(
    spills.count,
    `expected at least ${MIN_TILES} tiles to measure, saw ${spills.count}`,
  ).toBeGreaterThanOrEqual(MIN_TILES);

  expect(
    spills.out,
    `tile headers running past their tile: ${spills.out
      .map((s) => `${s.id} "${s.label}" overflows by ${s.by}px`)
      .join('; ')}`,
  ).toEqual([]);
});
