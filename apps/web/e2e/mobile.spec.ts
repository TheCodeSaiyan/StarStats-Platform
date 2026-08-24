import { test, expect } from '@playwright/test';
import { loginAs, resetScenario, scenarioFor, setScenario } from './helpers/api-mock';

/**
 * The product on a phone, measured on a phone.
 *
 * A REAL DEVICE CONTEXT, not a narrow desktop window. `pointer: coarse` gates
 * every 44px target rule in the stylesheet and it is driven by `hasTouch` — the
 * first version of this audit resized a desktop browser to 390px, read the
 * mouse-pointer rules, and reported a list of tap targets that do not exist on
 * a device. It also missed the opposite: under coarse the chrome row grows, and
 * that is what pushed the account control off the screen.
 *
 * WHAT THESE COVER, each from something that was actually wrong:
 *
 *   - The account control is ON the screen. It drew from x=493 to x=503 on a
 *     390px phone because the calibration pips, at 44px each, took 188px of a
 *     358px row. Sign out, Calibrate and Sharing live in that menu and it has
 *     no second home; the pips have one on /settings.
 *   - The range tabs are reachable. `.hp-top` is nowrap, so the strip was
 *     handed an 84px box for 226px of tabs and drew "All" at x=441 — painted,
 *     costed and untappable.
 *   - The page never scrolls sideways.
 *   - Tap targets clear the WCAG 2.5.8 (AA) 24px floor, with the standard
 *     exception for a link inline in a sentence, whose size is set by the
 *     line-height of the prose around it.
 */
test.use({ viewport: { width: 390, height: 844 }, hasTouch: true, isMobile: true });

const ROUTES = ['/me', '/me/travel', '/kb', '/settings', '/sharing', '/discover'];

test.describe('phone', () => {
  test.beforeEach(async ({ request }) => {
    await resetScenario(request);
    await setScenario(request, scenarioFor('phone'));
  });

  test('the chrome keeps the account menu on screen', async ({ page }) => {
    // Six routes, most of them a cold compile in `next dev`. The default 30s
    // test budget is sized for one navigation and this sweep exceeded it under
    // full-suite load while passing in isolation — the same shape every other
    // whole-app sweep in this directory carries `test.slow()` for.
    test.slow();
    await loginAs(page, { handle: 'TestPilot' });
    for (const url of ROUTES) {
      await page.goto(url, { waitUntil: 'domcontentloaded', timeout: 30_000 });
      await expect(page.locator('.hp-top[data-nav]')).toBeVisible({ timeout: 20_000 });
      await page.waitForTimeout(300);
      const box = await page.locator('.hp-acct').boundingBox();
      expect(box, `${url}: no account control`).not.toBeNull();
      expect(box!.x, `${url}: account starts off-screen`).toBeGreaterThanOrEqual(0);
      expect(
        Math.round(box!.x + box!.width),
        `${url}: account ends past the right edge`,
      ).toBeLessThanOrEqual(390);
    }
  });

  test('every range tab can be reached', async ({ page }) => {
    await loginAs(page, { handle: 'TestPilot' });
    await page.goto('/me', { waitUntil: 'domcontentloaded', timeout: 30_000 });
    const strip = page.locator('.hp-top .hp-rng');
    await expect(strip).toBeVisible();
    // Either every tab is already within the strip's box, or the strip scrolls
    // — a tab outside a box that CANNOT scroll is simply gone.
    const v = await page.evaluate(() => {
      const r = document.querySelector('.hp-top .hp-rng') as HTMLElement;
      const cs = getComputedStyle(r);
      const b = r.getBoundingClientRect();
      const tabs = [...r.children].map((t) => t.getBoundingClientRect());
      return {
        scrollable: cs.overflowX === 'auto' || cs.overflowX === 'scroll',
        canScroll: r.scrollWidth > r.clientWidth,
        allInside: tabs.every((t) => t.right <= b.right + 1),
        withinViewport: b.right <= window.innerWidth,
      };
    });
    expect(v.withinViewport, 'the strip itself must be on screen').toBe(true);
    expect(
      v.allInside || v.scrollable,
      'tabs overflow a strip that cannot scroll',
    ).toBe(true);
  });

  test('no page scrolls sideways', async ({ page }) => {
    test.slow();
    await loginAs(page, { handle: 'TestPilot' });
    for (const url of ROUTES) {
      await page.goto(url, { waitUntil: 'domcontentloaded', timeout: 30_000 });
      await page.waitForTimeout(400);
      const over = await page.evaluate(
        () => document.documentElement.scrollWidth - window.innerWidth,
      );
      expect(over, `${url} scrolls sideways by ${over}px`).toBeLessThanOrEqual(0);
    }
  });

  test('tap targets clear the 24px floor', async ({ page }) => {
    test.slow();
    await loginAs(page, { handle: 'TestPilot' });
    const failures: string[] = [];
    for (const url of [...ROUTES, '/auth/login', '/downloads']) {
      await page.goto(url, { waitUntil: 'domcontentloaded', timeout: 30_000 });
      await page.waitForTimeout(500);
      const small = await page.evaluate(() => {
        const out: string[] = [];
        document
          .querySelectorAll<HTMLElement>(
            'a[href], button, [role="button"], input, select, summary',
          )
          .forEach((el) => {
            const cs = getComputedStyle(el);
            if (cs.display === 'none' || cs.visibility === 'hidden') return;
            const b = el.getBoundingClientRect();
            if (b.width === 0 || b.height === 0) return;
            if (b.height >= 24 && b.width >= 24) return;
            // SVG shapes report the GEOMETRY box from getBoundingClientRect —
            // it excludes the stroke, which for a hit path IS the target. The
            // ring's segments measure 13px that way and are 24px in practice.
            if (el.namespaceURI === 'http://www.w3.org/2000/svg') return;
            // WCAG 2.5.8 exempts a target whose size is set by the line-height
            // of the sentence it sits in. Detected structurally: the parent
            // holds text of its own beyond this link.
            const p = el.parentElement;
            const own = (p?.textContent || '').replace(el.textContent || '', '').trim();
            if (p && own.length > 0 && cs.display.startsWith('inline')) return;
            out.push(
              `${el.tagName}.${String(el.className).slice(0, 24)} "${(el.textContent || '').trim().slice(0, 20)}" ${Math.round(b.width)}x${Math.round(b.height)}`,
            );
          });
        return [...new Set(out)];
      });
      for (const s of small) failures.push(`${url} — ${s}`);
    }
    expect(failures, failures.join('\n')).toEqual([]);
  });
});
