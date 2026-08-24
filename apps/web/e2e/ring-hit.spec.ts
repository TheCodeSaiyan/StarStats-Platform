import { test, expect } from '@playwright/test';
import { loginAs, resetScenario, scenarioFor, setScenario } from './helpers/api-mock';

/**
 * The ring is the projection's primary navigation, and it was not clickable.
 *
 * Every `.hp-layer` is `position: absolute; inset: 0`, so each depth layer
 * covers the whole stage and the last one in DOM order (callouts/panes, depth
 * 54) sat on top of the ring (20) and the core (36). Sampling 400 points around
 * the ring at 1440px and again at 390px, every single one returned
 * `DIV.hp-layer` and none returned a segment. The segments were painted, named,
 * focusable — and dead to a pointer, on desktop as much as on touch.
 *
 * Nothing caught it. The segments are `role="button"` with `tabIndex={0}` and
 * real key handlers, so they take a tab stop and activate from the keyboard;
 * the a11y sweep that walks tab stops passed the whole time. Only a pointer
 * ever found the layer.
 *
 * MEASURED BY HIT-TESTING THE PATH ITSELF, not by its bounding box:
 * `getBoundingClientRect` on an SVG path returns the GEOMETRY box and excludes
 * the stroke — and for a transparent hit path the stroke IS the target. That
 * box reads 13px wide for a band that is 24. Sampling `getPointAtLength` down
 * the arc and asking `elementFromPoint` what is on top measures what a finger
 * would actually reach.
 */
test.describe('ring', () => {
  test.beforeEach(async ({ request }) => {
    await resetScenario(request);
    await setScenario(request, scenarioFor('ring'));
  });

  test('a pointer reaches the segments, not the layer over them', async ({ page }) => {
    await loginAs(page, { handle: 'TestPilot' });
    for (const width of [1440, 390]) {
      await page.setViewportSize({ width, height: 900 });
      await page.goto('/me', { waitUntil: 'domcontentloaded', timeout: 30_000 });
      await expect(page.locator('.hp-seghit').first()).toBeAttached({ timeout: 20_000 });
      await page.waitForTimeout(400);
      const v = await page.evaluate(() => {
        const svg = document.querySelector('.hp-ringwrap svg') as SVGSVGElement;
        const m = svg.getScreenCTM()!;
        let hit = 0;
        let total = 0;
        const covering = new Set<string>();
        for (const h of document.querySelectorAll('.hp-seghit')) {
          const path = h as SVGPathElement;
          const len = path.getTotalLength();
          for (let i = 1; i < 10; i++) {
            const q = path.getPointAtLength((len * i) / 10);
            const sp = new DOMPoint(q.x, q.y).matrixTransform(m);
            const el = document.elementFromPoint(
              Math.round(sp.x),
              Math.round(sp.y),
            ) as Element | null;
            total++;
            if (el?.classList?.contains('hp-seghit')) hit++;
            else covering.add(el ? el.tagName : 'null');
          }
        }
        return { hit, total, covering: [...covering] };
      });
      expect(v.total, `${width}px: no segments to test`).toBeGreaterThan(10);
      // Most of every arc must be the topmost element. Not all of it: the
      // segment's own label sits over part of it, and that is fine — the label
      // passes its clicks through to the band beneath.
      expect(
        v.hit / v.total,
        `${width}px: ${v.total - v.hit}/${v.total} points covered by ${v.covering.join(',')}`,
      ).toBeGreaterThan(0.9);
    }
  });

  test('clicking a segment opens it', async ({ page }) => {
    // The behaviour, not the geometry. `data-mode` is how the stage records
    // that a segment took the reader somewhere.
    await loginAs(page, { handle: 'TestPilot' });
    await page.setViewportSize({ width: 1440, height: 900 });
    await page.goto('/me', { waitUntil: 'domcontentloaded', timeout: 30_000 });
    await expect(page.locator('.hp-seghit').first()).toBeAttached({ timeout: 20_000 });
    await page.waitForTimeout(400);
    const stage = page.locator('.hp-stage');
    await expect(stage).toHaveAttribute('data-mode', 'overview');
    const pt = await page.evaluate(() => {
      const svg = document.querySelector('.hp-ringwrap svg') as SVGSVGElement;
      const p = document.querySelectorAll('.hp-seghit')[2] as SVGPathElement;
      const q = p.getPointAtLength(p.getTotalLength() / 2);
      const sp = new DOMPoint(q.x, q.y).matrixTransform(svg.getScreenCTM()!);
      return { x: Math.round(sp.x), y: Math.round(sp.y) };
    });
    await page.mouse.click(pt.x, pt.y);
    await expect(stage).not.toHaveAttribute('data-mode', 'overview', {
      timeout: 10_000,
    });
  });
});
