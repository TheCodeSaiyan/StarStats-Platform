import { test, expect } from '@playwright/test';
import { loginAs, resetScenario, scenarioFor, setScenario } from './helpers/api-mock';

/**
 * The projection's drawing idiom, enforced on what actually renders.
 *
 * The system draws with lit hairlines on the void: square corners, no filled
 * panels. Flat components still render inside it through the bridge, and a flat
 * class with no bridge rule keeps its filled card and its rounded corners —
 * which is exactly what "it still looks like the old site" means.
 *
 * MEASURED ON COMPUTED STYLE, not on the stylesheet. A rule can exist and lose
 * to specificity, or be scoped to an ancestor the element no longer has (which
 * is how the widget canvas on `/u/[handle]` escaped all 38 bridge rules when it
 * moved out of `.hp-stage`). What the browser paints is the only fact.
 *
 * Tints are allowed and are not fills: the system uses beam at a few percent to
 * separate a panel from the void. The line is drawn at alpha, because that is
 * the difference between a tint and a card.
 */
const ROUTES = [
  { url: '/me', auth: true },
  { url: '/u/TestPilot', auth: true },
  { url: '/settings', auth: true },
  { url: '/sharing', auth: true },
  { url: '/kb/vehicle', auth: false },
  { url: '/contracts', auth: false },
  { url: '/downloads', auth: false },
  { url: '/orgs', auth: true },
  { url: '/admin', auth: true },
  { url: '/admin/users', auth: true },
  { url: '/admin/settings', auth: true },
];

test('nothing inside a projection is a filled or rounded box', async ({ page, request }) => {
  test.slow();
  await resetScenario(request);
  await setScenario(request, scenarioFor('idiom'));
  await loginAs(page, { handle: 'TestPilot', staffRoles: ['admin'] });

  const failures: string[] = [];
  for (const r of ROUTES) {
    // A generous goto budget, for a reason specific to these sweeps: this one
    // test visits every surface in the app, so most of its navigations are to
    // a route no other test has compiled yet. The config's 10s navigation
    // budget is sized for a warm route, and under full-suite parallelism a
    // cold one exceeded it — the sweep then failed on the harness rather than
    // on anything it measures. Nothing about what is asserted changes.
    await page.goto(r.url, { waitUntil: 'domcontentloaded', timeout: 30_000 });
    await page.waitForLoadState('networkidle').catch(() => {});
    await page.waitForTimeout(300);
    const bad = await page.evaluate(() => {
      const out: string[] = [];
      const root = document.querySelector('.ss-projection-root');
      if (!root) return ['no projection root'];
      root.querySelectorAll<HTMLElement>('*').forEach((el) => {
        const cs = getComputedStyle(el);
        const rect = el.getBoundingClientRect();
        // Under 10px in either axis it is a mark, not a panel: the lit status
        // dots, badges and rule caps are filled by definition and a 7px dot
        // cannot be "a card".
        if (rect.width < 10 || rect.height < 10) return;
        const id = `${el.tagName}.${String(el.className).slice(0, 44)}`;

        // Rounded corners. 50% is a dot; 0-2px is optical correction on a
        // hairline; anything else is the flat system's card.
        for (const corner of [cs.borderTopLeftRadius, cs.borderBottomRightRadius]) {
          const px = parseFloat(corner);
          if (corner.includes('%')) continue;
          if (px > 2) { out.push(`rounded ${corner}: ${id}`); break; }
        }

        // Filled panels. An opaque background inside the volume is a card.
        const m = cs.backgroundColor.match(/rgba?\(([^)]+)\)/);
        if (m) {
          const p = m[1].split(',').map(parseFloat);
          const a = p.length >= 4 ? p[3] : 1;
          // Media and code blocks legitimately paint themselves.
          const opaque = a > 0.5;
          const cls = String(el.className);
          // Each exemption is a thing that MUST be filled to do its job, named
          // individually so the list cannot quietly become a way to pass.
          const exempt =
            el.tagName === 'IMG' ||
            el.tagName === 'CANVAS' ||
            el.closest('[aria-hidden="true"]') !== null ||
            // The stage IS the void — it is the ground, not a panel on it.
            cls.includes('hp-stage') ||
            // A skip link must be readable over whatever it lands on.
            cls.includes('hp-skip') ||
            // A sticky pane header needs an opaque backing or the content
            // scrolls visibly through it.
            cls.includes('hp-phd') ||
            // Data marks: the fill IS the value. A heat cell, a meter bar and
            // a ring segment with no fill show nothing at all.
            cls.includes('heatcell') ||
            cls.includes('meter') ||
            cls.includes('__fill') ||
            cls.includes('hp-seg') ||
            cls.includes('hp-bar') ||
            // The QR needs white for a camera; skeletons are a placeholder
            // shimmer.
            cls.includes('hp-qr') ||
            cls.includes('skeleton');
          if (opaque && !exempt) out.push(`filled ${cs.backgroundColor}: ${id}`);
        }
      });
      return [...new Set(out)].slice(0, 10);
    });
    for (const b of bad) failures.push(`${r.url} — ${b}`);
  }
  expect(failures, failures.join('\n')).toEqual([]);
});
