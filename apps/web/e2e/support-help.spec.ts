import { expect, test } from '@playwright/test';
import { resetScenario, scenarioFor, setScenario, unauthorized } from './helpers/api-mock';

test.beforeEach(async ({ request }) => {
  await resetScenario(request);
});

// The whole point: an anonymous visitor reaches help, NOT a login bounce.
test('support_is_help_not_login', async ({ page, request }) => {
  await setScenario(request, scenarioFor('support_anon', { 'GET /v1/auth/me': unauthorized }));
  await page.goto('/support');
  await expect(page).not.toHaveURL(/\/auth\/login/);
  await expect(page.getByRole('heading', { level: 1 })).toBeVisible();
  // Scope to <main> — the chrome also has a "Docs" link with the same href, so
  // an unscoped locator hits Playwright's strict-mode violation.
  //
  // Scoped to `.hp-marketing`, the page's OWN body.
  //
  // This broke twice during the port and the two causes are different. First
  // every shell put `role="main"` on its outer wrapper, which contains the
  // `ChromeBar` — that was a defect and the landmark was moved onto
  // `#hp-content` rather than the test loosened. Then the CIG trademark plate
  // was added to every public surface, and it legitimately links to Docs from
  // INSIDE the landmark. So `main` is correct and still not the right scope
  // here; `marketing-capture.spec.ts` guards the landmark itself.
  //
  // Third scoping change, third distinct cause. The page now also carries the
  // DOCS INDEX — the grouped reference list from `Docs.jsx` — which links to
  // /docs by design, so "a link to docs inside the body" matches twice. The
  // one this test means is the page's own prose link, so the index's entries
  // are excluded by class rather than the assertion being loosened to `.first()`
  // (which would pass even if the prose link were deleted).
  await expect(
    page.locator('.hp-marketing a[href="/docs"]:not(.hp-docsindex__lk)'),
  ).toHaveCount(1);
});

// And it is NOT silently redirected to the payment page.
test('support_not_redirected_to_donate', async ({ page, request }) => {
  await setScenario(request, scenarioFor('support_anon2', { 'GET /v1/auth/me': unauthorized }));
  await page.goto('/support');
  await expect(page).not.toHaveURL(/\/donate/);
});
