import { expect, test } from '@playwright/test';
import {
  loginAs,
  notFound,
  publicSummaryShared,
  publicSummaryTestPilot,
  resetScenario,
  scenarioFor,
  setScenario,
  summaryWithEvents,
  timeline30Days,
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

test('a recipient sees the events behind the share, not just the aggregates', async ({
  page,
  request,
}) => {
  // Before `GET /v1/u/{handle}/events` existed this page could tell you a
  // pilot had logged thousands of events across a dozen types and still
  // not show you one of them — every friend-scoped read returned buckets.
  //
  // The assertion that actually differs is the SECOND one. Asserting a row
  // is present would pass on a feed that printed raw `actor_death`, which
  // is precisely what the headline rules exist to prevent; the row has to
  // read as a sentence, the way `/me/activity` and the tray render it.
  await loginAs(page, { handle: 'TestPilot' });
  await setScenario(request, {
    __id: 'shared_event_feed',
    routes: {
      'GET /v1/public/JohnSomeone/summary': notFound,
      'GET /v1/u/JohnSomeone/summary': publicSummaryShared,
      'GET /v1/u/JohnSomeone/events': {
        status: 200,
        body: {
          owner_handle: 'JohnSomeone',
          events: [
            {
              seq: 42,
              event_type: 'actor_death',
              event_timestamp: '2026-09-09T18:00:00Z',
              log_source: 'live',
              payload: { type: 'actor_death' },
            },
          ],
          next_before: null,
        },
      },
    },
  });

  await page.goto('/u/JohnSomeone');

  const rows = page.locator('.hp-lg-x');
  await expect(rows.first()).toBeVisible();

  const headline = rows.first().locator('.ev');
  await expect(headline).not.toHaveText('actor_death');
  await expect(headline).not.toHaveText('');
});

test('the detail docked below the volume can actually be reached', async ({
  page,
  request,
}) => {
  /**
   * `/u/[handle]` is the one projection that docks content BELOW its volume
   * (`.hp-volume-below`): the ring is the headline, and the pane carrying the
   * handle heading, the published/withheld scopes and the shared feed reads
   * underneath it.
   *
   * It could not be read at all. `projection-shell.css` locks the document for
   * every projection — `html, body { height: 100%; overflow: hidden }` — which
   * is right for the pane surfaces, because they scroll inside `.hp-settings`.
   * This surface has no inner scroller, so the lock applied to a page with
   * 2165px of content past the fold. Measured at 1440x900 before the fix:
   * `html.scrollHeight === html.clientHeight === 900`, `.hp-volume-below` at
   * y 1800-3065, and 3000px of wheel moved `window.scrollY` not one pixel.
   *
   * On top of that the offset was counted twice. `.hp-stage` is `height: 100%`
   * and IN FLOW, so it already occupies one viewport; `.hp-volume-below` then
   * added `margin-top: 100vh` on top, leaving a full blank screen between the
   * holo and the content even once the page could scroll.
   *
   * NOT a visibility assertion: Playwright's `toBeVisible` reads the box and
   * `visibility`, never whether an element is reachable, so the whole docked
   * pane asserted visible throughout. Geometry is the only thing that tells
   * the two renders apart.
   */
  await setScenario(request, {
    __id: 'public_below_volume_reachable',
    routes: { 'GET /v1/public/JohnSomeone/summary': publicSummaryShared },
  });

  await page.setViewportSize({ width: 1440, height: 900 });
  await page.goto('/u/JohnSomeone');
  await expect(page.locator('.hp-volume-below')).toBeAttached();

  // 1. The document has somewhere to scroll.
  const canScroll = await page.evaluate(
    () =>
      document.documentElement.scrollHeight >
      document.documentElement.clientHeight + 40,
  );
  expect(canScroll, 'the page must be scrollable to reach the dock').toBe(true);

  // 2. No blank viewport between the volume and the dock.
  const gap = await page.evaluate(() => {
    const stage = document.querySelector('.hp-stage:not([data-pending])')!;
    const below = document.querySelector('.hp-volume-below')!;
    return Math.round(
      below.getBoundingClientRect().top - stage.getBoundingClientRect().bottom,
    );
  });
  expect(gap, 'dock must start where the volume ends').toBeLessThanOrEqual(8);

  // 3. Scrolling with the wheel, as a reader would, brings it on screen.
  await page.mouse.move(720, 450);
  for (let i = 0; i < 6; i += 1) {
    await page.mouse.wheel(0, 500);
    await page.waitForTimeout(120);
  }
  const onScreen = await page.evaluate(() => {
    const r = document.querySelector('.hp-volume-below')!.getBoundingClientRect();
    return { top: Math.round(r.top), vh: window.innerHeight };
  });
  expect(
    onScreen.top,
    `dock is at y=${onScreen.top} in a ${onScreen.vh}px viewport after scrolling`,
  ).toBeLessThan(onScreen.vh);
});

/** Owner viewing their own profile, with data behind every default widget. */
const OWNER_PROFILE_ROUTES = {
  'GET /v1/public/TestPilot/summary': publicSummaryTestPilot,
  'GET /v1/u/TestPilot/summary': summaryWithEvents,
  'GET /v1/me/summary': summaryWithEvents,
  'GET /v1/me/timeline': timeline30Days,
  'GET /v1/u/TestPilot/timeline': timeline30Days,
  'GET /v1/users/TestPilot/sessions': {
    status: 200,
    body: {
      sessions: [
        {
          id: 's1',
          started_at: '2026-09-08T14:00:00Z',
          ended_at: '2026-09-08T16:30:00Z',
          event_count: 42,
        },
      ],
    },
  },
  'GET /v1/public/TestPilot/share-scopes': {
    status: 200,
    body: {
      combat_mission: true,
      economy: false,
      travel: false,
      records: true,
      recent_activity: false,
    },
  },
  'GET /v1/users/me/profile-layout': { status: 200, body: { layout: null } },
  'GET /v1/public/TestPilot/rsi-profile': { status: 404, body: { error: 'not_found' } },
  'GET /v1/public/TestPilot/rsi-orgs': { status: 200, body: { orgs: [] } },
};

test('the profile body is drawn in the projection, not the flat widget canvas', async ({
  page,
  request,
}) => {
  /**
   * `/u/[handle]` was the last surface in the app still rendering
   * `WidgetCanvas` — the flat-era 24-column free grid — inside the projection.
   * The port (`aae18ac`) redrew ~45 page bodies and deliberately left this one
   * behind: `me/page.tsx` records that "the public profile deliberately did
   * NOT come along". Nobody noticed, because the dock it lives in was
   * unreachable until v0.1.38.
   *
   * The assertion is which SYSTEM drew the body, because both render tiles
   * that report `visible` and carry the same text. `.hud-freegrid` is the flat
   * canvas's own container and exists nowhere in the projection language.
   */
  await setScenario(request, {
    __id: 'public_body_is_projection',
    routes: OWNER_PROFILE_ROUTES,
  });
  await loginAs(page, { handle: 'TestPilot' });
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.goto('/u/TestPilot');

  const dock = page.locator('.hp-volume-below');
  await expect(dock).toBeAttached();
  // Projection planes drew the body...
  await expect(dock.locator('.hp-plane').first()).toBeAttached();
  // ...and the flat grid is gone from the page entirely.
  await expect(page.locator('.hud-freegrid')).toHaveCount(0);

  // Half the widget set builds a CALLOUT, not a plane — `sessions` among them,
  // and the flat canvas drew it as a tile. Rendering only `elements.planes`
  // dropped every one of those silently, which no count of planes would catch.
  await expect(
    dock.getByText(/session/i).first(),
    'widget callouts must reach the dock, not just planes',
  ).toBeVisible();
});

test('the owner can still arrange the profile layout', async ({
  page,
  request,
}) => {
  /**
   * View mode is projection-native for EVERY reader including the owner —
   * the pane's own context line claims "your profile, as others see it", and
   * showing the owner a different design would make that a lie.
   *
   * Arranging is therefore a distinct mode behind `?arrange=1` rather than a
   * client toggle: a URL-driven mode stays shareable and back-button correct,
   * the same reasoning `RangeTabs` records for `?range=`, and the server can
   * render the editor in place of the dock.
   *
   * The editor is the projection's own (`.hp-layout`), not the flat 24-column
   * drag grid it replaced — that grid edited geometry this surface stopped
   * reading when the body became a plane stack, so an owner could drag a tile
   * somewhere nothing would render it.
   */
  await setScenario(request, {
    __id: 'public_owner_arrange',
    routes: OWNER_PROFILE_ROUTES,
  });
  await loginAs(page, { handle: 'TestPilot' });
  await page.setViewportSize({ width: 1440, height: 900 });

  // The affordance exists in view mode...
  await page.goto('/u/TestPilot');
  const arrange = page.getByRole('link', { name: /arrange/i });
  await expect(arrange).toBeVisible();

  // ...and it reaches the editor.
  await arrange.click();
  await expect(page).toHaveURL(/arrange=1/);
  await expect(page.locator('.hp-layout')).toBeVisible();
  // ...offering the profile's own elements, with the widget the dock draws
  // by default among them.
  await expect(
    page.locator('.hp-layout').getByText(/sessions/i).first(),
  ).toBeVisible();
  // The flat grid is gone from this surface entirely, editor included.
  await expect(page.locator('.hud-freegrid')).toHaveCount(0);
});

test('a visitor is never offered the arrange mode', async ({ page, request }) => {
  // `?arrange=1` is owner-only and ignored for anyone else — a visitor who
  // types the URL gets the ordinary read-only profile, not an editor over
  // someone else's layout.
  await setScenario(request, {
    __id: 'public_visitor_no_arrange',
    routes: { 'GET /v1/public/JohnSomeone/summary': publicSummaryShared },
  });
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.goto('/u/JohnSomeone?arrange=1');

  await expect(page.locator('.hud-freegrid')).toHaveCount(0);
  await expect(page.locator('.hp-layout')).toHaveCount(0);
  await expect(page.getByRole('link', { name: /arrange/i })).toHaveCount(0);
});

test('the ring fills a tall volume instead of leaving a dead band', async ({
  page,
  request,
}) => {
  /**
   * `--hp-ring` is a fixed 560px on this surface while the volume is `100svh`,
   * so the taller the window the more empty space sits between the ring and
   * the lens rail. Reported from a real screen at ~1350px of content height:
   * roughly 350px of nothing below the ring. It never showed up in testing
   * because a 900px viewport is about the size the fixed value was chosen for.
   *
   * The brand surface already scales its ring (`min(760px, 72vw)`); this is
   * the same idea for the volume, bounded so a short window still shrinks.
   *
   * Measured as the GAP, not the ring size — the ring being "big enough" is
   * meaningless on its own, and the complaint was about the space under it.
   */
  await setScenario(request, {
    __id: 'public_ring_tall_viewport',
    routes: { 'GET /v1/public/JohnSomeone/summary': publicSummaryShared },
  });
  await page.setViewportSize({ width: 1600, height: 1300 });
  await page.goto('/u/JohnSomeone');

  const ring = page.locator('.hp-ringwrap');
  await expect(ring).toBeVisible();

  const m = await page.evaluate(() => {
    const r = document.querySelector('.hp-ringwrap')!.getBoundingClientRect();
    const rail = document.querySelector('.hp-railstack, .hp-lens')!;
    return {
      gap: Math.round(rail.getBoundingClientRect().top - r.bottom),
      vh: window.innerHeight,
    };
  });

  // Proportional, not a tuned constant: the complaint scales with the window,
  // so the bar has to as well. A fifth of the viewport is the line between
  // breathing room and a hole. The fixed 560px ring left 300px here (23%);
  // scaled it leaves 200px (15%), and the space above the ring matches it.
  expect(
    m.gap,
    `${m.gap}px of empty volume under the ring in a ${m.vh}px viewport`,
  ).toBeLessThan(m.vh * 0.2);
});

test('the callout field stands down rather than clipping in a short window', async ({
  page,
  request,
}) => {
  /**
   * `CALLOUT_SLOTS` are fixed pixel depths in a stage that is 100svh, so the
   * field does not shrink with the window — past a point the bottom row is
   * simply cut off by the stage floor. Measured before the guard existed: at
   * 1200x560 the two deepest callouts drew to y=574 and y=566 against a stage
   * ending at 560.
   *
   * The assertion is the floor, not visibility — a clipped callout is still
   * `visibility: visible`, it is the stage that cuts it. Above the guard every
   * callout must sit inside the stage; below it the field must be ABSENT
   * rather than present-and-clipped, which is the same standing-down the
   * width guard has always done.
   */
  await setScenario(request, {
    __id: 'public_callouts_short_window',
    routes: {
      'GET /v1/public/JohnSomeone/summary': {
        status: 200,
        body: {
          claimed_handle: 'JohnSomeone',
          total: 325888,
          supporter: 'gold',
          by_type: [
            { event_type: 'attached_gear', count: 99513 },
            { event_type: 'mission_objective', count: 60112 },
            { event_type: 'stowed_ship', count: 40233 },
            { event_type: 'hud_notice', count: 31004 },
            { event_type: 'loaded_planet', count: 28777 },
            { event_type: 'login', count: 20111 },
            { event_type: 'death', count: 9044 },
          ],
        },
      },
      'GET /v1/public/JohnSomeone/share-scopes': {
        status: 200,
        body: {
          combat_mission: true,
          economy: false,
          travel: false,
          records: true,
          recent_activity: false,
        },
      },
    },
  });

  for (const [width, height] of [
    [1920, 1080],
    [1600, 1300],
    [1440, 900],
    [1280, 800],
    [1280, 700],
    [1200, 560],
  ] as const) {
    await page.setViewportSize({ width, height });
    await page.goto('/u/JohnSomeone');
    await expect(page.locator('.hp-ringwrap').first()).toBeVisible();

    const seen = await page.evaluate(() => {
      const stage = document
        .querySelector('.hp-stage:not([data-pending])')!
        .getBoundingClientRect();
      const field = document.querySelector('.hp-cos');
      const drawn = field && getComputedStyle(field).display !== 'none';
      const overflowing = !drawn
        ? []
        : Array.from(document.querySelectorAll('.hp-co'))
            .map((c) => ({
              txt: (c.textContent ?? '').trim().slice(0, 16),
              b: c.getBoundingClientRect(),
            }))
            .filter(
              ({ b }) =>
                b.bottom > stage.bottom ||
                b.top < stage.top ||
                b.left < stage.left ||
                b.right > stage.right,
            )
            .map(({ txt, b }) => `${txt} ${Math.round(b.top)}-${Math.round(b.bottom)}`);
      return { drawn, overflowing, floor: Math.round(stage.bottom) };
    });

    expect(
      seen.overflowing,
      `callouts past the stage edge at ${width}x${height} (floor ${seen.floor})`,
    ).toEqual([]);

    // Below the guard it must stand down, not merely happen to fit.
    if (height <= 620) expect(seen.drawn, `field drawn at ${width}x${height}`).toBe(false);
  }
});
