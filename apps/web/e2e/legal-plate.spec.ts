/**
 * The CIG trademark line, on every public surface.
 *
 * NOT a capture spec — this one stays. The design system's rule is that every
 * static surface carries the disclaimer verbatim, and the projection port
 * BROKE it silently: the flat shell rendered `.site-footer` from
 * `layout.tsx`'s signed-out branch, so every public page had the notice for
 * free. As each page became a projection, `projection-shell.css` hid that
 * footer — and for several commits only the landing page, which had been given
 * a plate of its own, still showed it. Nothing failed. Nothing warned.
 *
 * The words are the PRODUCT's, not the kit's `CIG_DISCLAIMER`: they name
 * Squadron 42 and assert the Cloud Imperium Rights copyright over
 * specifications as well as names. Asserted here so a future edit that reaches
 * for the component's shorter default is a failing test rather than a quiet
 * rewrite of a legal notice.
 *
 * It earned its place on the first run: `/contracts` is public, wraps
 * `AppSurface` directly rather than one of the shared shells, and had no plate.
 * Nothing else in the suite would have noticed.
 */
import { test, expect } from '@playwright/test';
import {
  loginAs,
  resetScenario,
  scenarioFor,
  setScenario,
} from './helpers/api-mock';

const PUBLIC_SURFACES = [
  '/',
  '/features',
  '/about',
  '/trust',
  '/privacy',
  '/terms',
  '/docs',
  '/guides',
  '/changelog',
  '/kb',
  '/kb/vehicle',
  '/discover',
  '/downloads',
  '/contracts',
  '/auth/login',
  '/auth/signup',
] as const;

test.beforeEach(async ({ request, page }) => {
  await resetScenario(request);
  await setScenario(request, scenarioFor('legal-plate', {
    'GET /v1/admin/submissions/queue': { status: 200, body: { items: [], has_more: false } },
  }));
  await page.setViewportSize({ width: 1440, height: 900 });
});

/**
 * SIGNED-IN surfaces carry it too. The flat `.ss-app-footer` did — its own
 * comment cites "Brand book §11 compliance: About + Fankit + Fandom-FAQ
 * outbound links plus the attribution chip are reachable from every signed-in
 * surface" — and `projection-shell.css` hides it. Both audiences lost the
 * block; both get it back.
 */
const SIGNED_IN_SURFACES = ['/me', '/settings', '/sharing', '/orgs', '/admin'] as const;

for (const route of PUBLIC_SURFACES) {
  test(`${route} carries the trademark line`, async ({ page }) => {
    await page.goto(route);
    const legal = page.locator('.hp-legal');
    await expect(legal).toHaveCount(1);
    // The specific claims, not merely "some footer exists".
    await expect(legal).toContainText('Not affiliated with Cloud Imperium Games');
    await expect(legal).toContainText('Squadron 42');
    await expect(legal).toContainText('Cloud Imperium Rights');
  });
}

for (const route of SIGNED_IN_SURFACES) {
  test(`${route} carries the trademark line, signed in`, async ({ page }) => {
    await loginAs(page, { handle: 'TestPilot', staffRoles: ['admin'] });
    await page.goto(route);
    const legal = page.locator('.hp-legal');
    await expect(legal).toHaveCount(1);
    await expect(legal).toContainText('Cloud Imperium Rights');
    // The two OUTBOUND links §11 names by name.
    await expect(legal.getByRole('link', { name: 'RSI Fankit' })).toHaveAttribute(
      'href',
      'https://robertsspaceindustries.com/en/fankit',
    );
    await expect(legal.getByRole('link', { name: 'Fandom FAQ' })).toHaveCount(1);
  });
}

test('the unverified-email banner is visible, not merely rendered', async ({
  page,
}) => {
  // It was hidden with the flat chrome for the whole port. It is not chrome:
  // "claim it before someone else can" is a security nudge with a deadline.
  await loginAs(page, { handle: 'TestPilot', emailVerified: false });
  await page.goto('/me');
  const banner = page.locator('.unverified-banner');
  await expect(banner).toHaveCount(1);
  await expect(banner).toBeVisible();
  await expect(banner).toContainText('Email unverified');
  await expect(banner.getByRole('link', { name: /resend/i })).toBeVisible();
});

test('a verified reader is not nagged', async ({ page }) => {
  await loginAs(page, { handle: 'TestPilot', emailVerified: true });
  await page.goto('/me');
  await expect(page.locator('.unverified-banner')).toHaveCount(0);
});

test('an inbound share is announced on every signed-in surface', async ({
  page,
  request,
}) => {
  // The flat `AccountMenu` badged this on every page, fed by one fetch in the
  // root layout. Each projection surface builds its own chrome, so restoring it
  // per-shell would have put the badge on some pages and not others — which
  // teaches a reader the wrong thing about where notifications live. It comes
  // from a context the layout provides, so it is genuinely everywhere.
  await setScenario(
    request,
    scenarioFor('inbound-badge', {
      'GET /v1/me/shared-with-me': {
        status: 200,
        body: {
          shared_with_me: [
            { owner_handle: 'Alice' },
            { owner_handle: 'Bob', expires_at: '2099-01-01T00:00:00Z' },
            // Expired: counted in the LIST but never in the badge. The layout's
            // own reasoning — an expired badge is noise and never clears.
            { owner_handle: 'Cass', expires_at: '2020-01-01T00:00:00Z' },
          ],
        },
      },
    }),
  );
  await loginAs(page, { handle: 'TestPilot' });

  for (const route of ['/me', '/settings', '/kb', '/orgs']) {
    await page.goto(route);
    const badge = page.locator('.hp-acct .hp-badge').first();
    await expect(badge, route).toBeVisible();
    await expect(badge, route).toHaveText('2');
  }
});

test('no inbound shares means no badge', async ({ page, request }) => {
  await setScenario(
    request,
    scenarioFor('inbound-none', {
      'GET /v1/me/shared-with-me': { status: 200, body: { shared_with_me: [] } },
    }),
  );
  await loginAs(page, { handle: 'TestPilot' });
  await page.goto('/me');
  await expect(page.locator('.hp-badge')).toHaveCount(0);
});

test('the legal plate can be followed to the full text', async ({ page }) => {
  // The plate summarises a longer position and shows on every surface. Asserted
  // as a WORKING link, not just present markup: a read-more that 404s is worse
  // than none.
  await page.goto('/kb');
  const more = page.locator('.hp-legal').getByRole('link', {
    name: /read the full terms/i,
  });
  await expect(more).toBeVisible();
  await more.click();
  await expect(page).toHaveURL(/\/terms$/);
  await expect(page.locator('h1')).toHaveText('Terms of Service');
});

test('the legal documents reach each other', async ({ page }) => {
  // Before the index, each was reachable only from a footer link and led
  // nowhere else.
  await page.goto('/privacy');
  const index = page.locator('.hp-legalindex');
  await expect(index).toHaveCount(1);
  await expect(index.locator('[aria-current="page"]')).toHaveText('Privacy');
  await index.getByRole('link', { name: 'Trust' }).click();
  await expect(page).toHaveURL(/\/trust$/);
  await expect(
    page.locator('.hp-legalindex [aria-current="page"]'),
  ).toHaveText('Trust');
});
