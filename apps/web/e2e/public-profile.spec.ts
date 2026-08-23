import { expect, test } from '@playwright/test';
import {
  loginAs,
  notFound,
  publicSummaryShared,
  resetScenario,
  scenarioFor,
  setScenario,
} from './helpers/api-mock';

test.beforeEach(async ({ request }) => {
  await resetScenario(request);
});

test('public_profile_renders_when_visible', async ({ page, request }) => {
  await setScenario(request, {
    __id: 'public_visible',
    routes: {
      'GET /v1/public/JohnSomeone/summary': publicSummaryShared,
    },
  });

  await page.goto('/u/JohnSomeone');

  await expect(
    page.getByRole('heading', { name: 'JohnSomeone' }),
  ).toBeVisible();
  // The viewer's relationship to the profile. It used to sit in an
  // `InstrumentStrip` header; the projection states it as the pane's context
  // line, which is the same claim in the shape the system has for it.
  await expect(page.locator('.hp-phd .ctx')).toHaveText('Public projection');
  // The shared total, in the core readout. Scoped to `.hp-core .n` and not to
  // any element containing "42": the footer's "Squadron 42™" trademark line
  // matches that, and so do the readout's own chromatic fringe layers.
  await expect(page.locator('.hp-core .n')).toHaveText('42');
});

test('public_profile_404_shows_generic_message', async ({ page, request }) => {
  await setScenario(request, {
    __id: 'public_404',
    routes: {
      'GET /v1/public/Phantom/summary': notFound,
    },
  });

  // No session cookie -> the page can't fall back to the friend path,
  // so 404 surfaces the generic "not available" view.
  await page.goto('/u/Phantom');

  await expect(
    page.getByRole('heading', { name: 'Profile not available' }),
  ).toBeVisible();
  await expect(
    page.getByText(/doesn.t exist.*isn.t public.*hasn.t been shared/),
  ).toBeVisible();
});

test('public_profile_falls_back_to_friend_view_when_logged_in', async ({
  page,
  request,
}) => {
  await loginAs(page, { handle: 'TestPilot' });
  await setScenario(request, {
    __id: 'public_friend_fallback',
    routes: {
      'GET /v1/public/JohnSomeone/summary': notFound,
      'GET /v1/u/JohnSomeone/summary': publicSummaryShared,
    },
  });

  await page.goto('/u/JohnSomeone');

  await expect(
    page.getByRole('heading', { name: 'JohnSomeone' }),
  ).toBeVisible();
  await expect(page.locator('.hp-phd .ctx')).toHaveText('Shared with you');
});

test('every reader is told what this pilot publishes and withholds', async ({
  page,
  request,
}) => {
  // `Profile.jsx` states both halves and says why: "a public profile must never
  // imply data it is not allowed to show." The product said neither, so a
  // reader could not tell a quiet pilot from a private one.
  //
  // THIS TEST REPLACES ONE THAT ASSERTED THE OPPOSITE for visitors. The old
  // rule — owner-only — came from deriving the answer from the profile LAYOUT,
  // which a visitor is not served. But the page already fetched
  // `/v1/public/{handle}/share-scopes`, an unauthenticated endpoint carrying
  // the pilot's own switches, and passed it to the widget canvas without
  // reading it. It is the pilot's decision, so it can be stated to anyone.
  await setScenario(request, scenarioFor('profile_visitor_scopes'));
  await loginAs(page, { handle: 'SomeoneElse' });
  await page.goto('/u/TestPilot');

  const published = page.locator('.hp-plane', { hasText: 'Published' }).first();
  await expect(published).toBeVisible();
  const shown = await published.locator('.hp-rw .nm').allTextContents();

  const withheldPane = page.locator('.hp-plane', { hasText: 'Not published' });
  const withheldText = (await withheldPane.count())
    ? await withheldPane.innerText()
    : '';

  // Every scope is accounted for, one way or the other. A scope in neither
  // list is the failure this exists to catch: it reads as absent data rather
  // than as a withheld choice.
  for (const scope of [
    'Combat & Missions',
    'Economy',
    'Travel',
    'Records',
    'Recent activity',
  ]) {
    expect(
      shown.includes(scope) || withheldText.includes(scope),
      `${scope} is neither published nor withheld`,
    ).toBe(true);
  }
  // The fixture publishes three and withholds two, so both halves are real
  // here rather than one being trivially empty.
  expect(shown.length).toBe(3);
  expect(withheldText).toContain('Economy');
});

test('a failed scope read is never reported as "publishes nothing"', async ({
  page,
  request,
}) => {
  // The load-bearing half now. `fetchShareScopes` falls back to
  // DEFAULT_SHARE_SCOPES — every scope false — when the endpoint does not
  // answer. Rendering that verbatim would tell every reader this pilot
  // publishes nothing, on the strength of a network error, and would do it on
  // the one page whose entire job is stating someone's privacy choices.
  await setScenario(
    request,
    scenarioFor('profile_scopes_unavailable', {
      'GET /v1/public/TestPilot/share-scopes': { status: 503, body: {} },
    }),
  );
  await loginAs(page, { handle: 'SomeoneElse' });
  await page.goto('/u/TestPilot');

  await expect(
    page.getByText('Could not read what this pilot publishes'),
  ).toBeVisible();
  await expect(page.getByText('Nothing is published')).toHaveCount(0);
  await expect(
    page.locator('.hp-plane', { hasText: 'Not published' }),
  ).toHaveCount(0);
});

test('the public profile draws a real distribution, not a placeholder split', async ({
  page,
  request,
}) => {
  // The kit gives the ring one equal segment per published lens. Equal
  // segments draw a distribution that does not exist, and every other ring in
  // this product is proportional — so this one carries `by_type`, which is
  // real.
  //
  // ASSERTED ON ARC LENGTH, not on the ring being present. A `1/n` split and a
  // real distribution both render the same element count with the same
  // classes; the only thing that differs is how long each arc is. The fixture
  // is 30 logins to 12 deaths, so two equal arcs is precisely the regression.
  await setScenario(
    request,
    scenarioFor('profile_ring', {
      'GET /v1/public/TestPilot/summary': {
        status: 200,
        body: {
          claimed_handle: 'TestPilot',
          total: 42,
          by_type: [
            { event_type: 'login', count: 30 },
            { event_type: 'death', count: 12 },
          ],
        },
      },
    }),
  );
  await page.goto('/u/TestPilot');

  const segs = page.locator('path.hp-seg');
  await expect(segs).toHaveCount(2);
  const lengths = await segs.evaluateAll((els) =>
    els.map((e) => (e as unknown as SVGPathElement).getTotalLength()),
  );
  expect(lengths.every((l) => l > 0)).toBe(true);
  // 30:12 is 2.5:1. Anything near 1:1 means the shares were not real.
  const ratio = Math.max(...lengths) / Math.min(...lengths);
  expect(ratio).toBeGreaterThan(2);
});

test('the profile pane is actually painted, not just present', async ({
  page,
  request,
}) => {
  // THE ASSERTION THAT WAS MISSING. `.hp-pane` is `opacity: 0;
  // pointer-events: none` until the stage is in `data-mode="detail"`, so the
  // first version of this screen rendered the handle, the published scopes and
  // the entire widget canvas at zero opacity inside an overview volume.
  //
  // Every existing check passed on it. `toBeVisible()` reads the bounding box
  // and `visibility` and does NOT read opacity; `toHaveText` does not care
  // either. What caught it was a `hover` timing out because a stage layer
  // swallowed the pointer — by accident, in an unrelated spec.
  //
  // So: computed opacity, effective pointer-events, and a real hit test at the
  // element's own centre.
  await setScenario(request, scenarioFor('profile_pane_painted'));
  await page.goto('/u/TestPilot');

  const pane = page.locator('.hp-pane').first();
  await expect(pane).toBeVisible();
  // It docks BELOW the volume, so it starts outside the viewport and
  // `elementFromPoint` would answer about a point that is not on screen.
  await pane.scrollIntoViewIfNeeded();

  const report = await pane.evaluate((el) => {
    // Opacity multiplies through ancestors for painting purposes, so walk up
    // rather than reading the element alone.
    let node: Element | null = el;
    let opacity = 1;
    while (node) {
      opacity *= Number(getComputedStyle(node).opacity);
      node = node.parentElement;
    }
    // Probe a point that is inside BOTH the element and the viewport. The
    // pane is taller than the window, so `scrollIntoViewIfNeeded` can align
    // its bottom edge and leave `rect.top` above the fold — `elementFromPoint`
    // then answers `null` and the test fails for the wrong reason.
    const r = el.getBoundingClientRect();
    const y = Math.min(Math.max(r.top + 20, 8), window.innerHeight - 8);
    const at = document.elementFromPoint(r.left + r.width / 2, y);
    return {
      opacity,
      inert: getComputedStyle(el).pointerEvents === 'none',
      hit: Boolean(at && el.contains(at)),
    };
  });

  expect(report.inert).toBe(false);
  expect(report.opacity).toBeGreaterThan(0.9);
  // Nothing overlays it: a click at the pane's own centre reaches the pane.
  expect(report.hit).toBe(true);
});
