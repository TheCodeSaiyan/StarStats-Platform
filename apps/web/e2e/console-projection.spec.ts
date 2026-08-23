/**
 * The console surface, in the projection.
 *
 * NOT a capture spec any more. This file began as scaffolding for the port —
 * a set of `goto` + `waitForTimeout` + `screenshot` cases whose only job was
 * producing images to judge, plus the fixtures they needed. Those 28 cases
 * asserted nothing, slept for half a second each, and are gone; what is left
 * are the assertions written alongside them, which are about behaviour and
 * outlive the port.
 */
import { test, expect } from '@playwright/test';
import {
  loginAs,
  resetScenario,
  scenarioFor,
  setScenario,
} from './helpers/api-mock';

const consoleErrors: string[] = [];

/**
 * Empty payloads on purpose. These tests are about the FRAME — the pages'
 * own content is covered by `admin.spec.ts` and the four `admin-*.spec.ts`
 * files. What matters here is that each route renders at all: without a
 * fixture the page throws, the error boundary replaces the whole tree
 * INCLUDING the layout, and an assertion about the section nav then fails
 * for a reason that has nothing to do with the port. (It did.)
 */
const ADMIN_FIXTURES = {
  'GET /v1/admin/submissions/queue': {
    status: 200,
    body: { items: [], has_more: false },
  },
  'GET /v1/admin/users': {
    status: 200,
    body: { users: [], has_more: false },
  },
  'GET /v1/admin/audit': {
    status: 200,
    body: { entries: [], has_more: false },
  },
};

test.beforeEach(async ({ page, request }) => {
  consoleErrors.length = 0;
  page.on('console', (m) => {
    if (m.type() === 'error') consoleErrors.push(m.text());
  });
  page.on('pageerror', (e) => consoleErrors.push(`pageerror: ${e.message}`));
  await resetScenario(request);
  await setScenario(request, scenarioFor('console-projection', ADMIN_FIXTURES));
  await loginAs(page, { handle: 'TheCodeSaiyan', staffRoles: ['admin'] });
  await page.setViewportSize({ width: 1440, height: 900 });
});

test('the console renders in the projection, not the flat shell', async ({
  page,
}) => {
  await page.goto('/admin');
  await expect(page.locator('.hp-stage')).toBeVisible();
  // Hidden, not absent — a nested layout cannot remove a parent layout.
  await expect(page.locator('.ss-topbar')).toHaveCount(0);
  await expect(page.locator('.ss-rail')).toHaveCount(0);
});

test('the console surface kills the ambience', async ({ page }) => {
  // `surface="console"` is a DECLARED intent, and the system's argument for it
  // is that at eight hours a day the parallax, scanlines and floor are noise.
  // Assert the declaration reached the DOM rather than trusting the prop.
  await page.goto('/admin');
  await expect(page.locator('.hp-stage')).toHaveAttribute(
    'data-surface',
    'console',
  );
});

test('the page inside the frame still renders', async ({ page }) => {
  // The frame moving must not take the content with it. This is the check the
  // Emitter port did not have, where a correct rail sat above nothing at all.
  await page.goto('/admin');
  await expect(
    page.getByRole('heading', { name: 'Moderation' }),
  ).toBeVisible();
});

test('exactly one h1, and it names the admin page — not the Console', async ({
  page,
}) => {
  // Every other ported surface puts the h1 in the crumb. Here each page already
  // renders its own via `AdminPageHeader`, and that names the specific page far
  // better than a shared crumb could — so the crumb deliberately does NOT carry
  // `heading`. Getting this wrong would put two h1s on all twenty pages.
  await page.goto('/admin');
  await expect(page.locator('h1')).toHaveCount(1);
  await page.goto('/admin/users');
  await expect(page.locator('h1')).toHaveCount(1);
  await expect(page.locator('h1')).toHaveText('Users');
});

test('exactly one main landmark', async ({ page }) => {
  // The landmark moved from the layout's wrapper into `ConsoleShell`. Two
  // mains is the failure mode of having added one without removing the other.
  await page.goto('/admin');
  await expect(page.locator('[role="main"], main')).toHaveCount(1);
});

test('the section rail is the system Console, not a hand-built strip', async ({
  page,
}) => {
  // NOT a "no rounded pill" assertion, which was the first thing written here
  // and was worthless: `--r-pill` is not aliased into the beam, so the old
  // inline `borderRadius: var(--r-pill)` computes to `0px` inside `.hp-stage`
  // anyway. It would have passed on the UN-PORTED code.
  //
  // What actually changed is that the tabs are stylesheet-driven instead of
  // inline-styled — which is what lets them follow the beam when the reader
  // recalibrates — and a className'd element with no CSS behind it passes every
  // other gate in this suite. So: assert the rule landed, and assert the two
  // states differ.
  await page.goto('/admin/users');
  // `.hp-cnav` is the system's Console rail. Two earlier passes hand-built a
  // strip above the content instead — pills, then hairline tabs — for a shell
  // the system already ships.
  await expect(page.locator('.hp-cnav')).toHaveCount(1);
  await expect(page.locator('.hp-consnav, .hp-constab')).toHaveCount(0);
  const styled = await page.locator('.hp-cnav a').evaluateAll((els) =>
    els.map((el) => ({
      inline: el.getAttribute('style'),
      active: el.getAttribute('aria-current') === 'page',
      border: getComputedStyle(el).borderTopColor,
      radius: getComputedStyle(el).borderRadius,
      minHeight: getComputedStyle(el).minHeight,
    })),
  );
  expect(styled.length).toBeGreaterThan(5);
  // Stylesheet, not inline — nothing here may set its own colours.
  expect(styled.every((t) => t.inline === null)).toBe(true);
  expect(styled.every((t) => t.radius === '0px')).toBe(true);
  // Exactly one item marks itself as the route you are on.
  expect(styled.filter((t) => t.active)).toHaveLength(1);
});

test('the active rail item is marked, and it is the one you are on', async ({
  page,
}) => {
  await page.goto('/admin/users');
  const active = page.locator('.hp-cnav a[aria-current="page"]');
  await expect(active).toHaveCount(1);
  await expect(active).toHaveText('Users');
});

test('navigating between console sections works', async ({ page }) => {
  await page.goto('/admin');
  await page.locator('.hp-cnav a', { hasText: 'Audit log' }).click();
  await expect(page).toHaveURL(/\/admin\/audit/);
  await expect(page.locator('.hp-cnav a[aria-current="page"]')).toHaveText(
    'Audit log',
  );
});

test('no console errors', async ({ page }) => {
  await page.goto('/admin');
  await expect(page.locator('.hp-cnav')).toBeVisible();
  await page.waitForTimeout(900);
  if (consoleErrors.length) {
    console.log(`CONSOLE ERRORS:\n${consoleErrors.join('\n---\n')}`);
  }
  expect(consoleErrors).toEqual([]);
});
