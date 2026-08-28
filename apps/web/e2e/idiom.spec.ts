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
const CORE_ROUTES = [
  { url: '/me', auth: true },
  { url: '/u/TestPilot', auth: true },
  { url: '/settings', auth: true },
  { url: '/sharing', auth: true },
  { url: '/kb/vehicle', auth: false },
  { url: '/contracts', auth: false },
  { url: '/downloads', auth: false },
  { url: '/orgs', auth: true },
];

/**
 * The admin console, as its own sweep.
 *
 * Split from the core surfaces for a harness reason, not a semantic one: one
 * test walking all 28 routes exceeded even the tripled `test.slow()` budget,
 * and a timeout measures nothing. Separately, an admin regression no longer
 * masks a core one — the first failure used to end the whole sweep.
 */
const ADMIN_ROUTES = [
  { url: '/admin', auth: true },
  { url: '/admin/users', auth: true },
  { url: '/admin/settings', auth: true },
  // The rest of the console. Admin was flagged in the overnight review as
  // "not written in the system's components", but that review also records
  // why a JSX-name scan measures nothing — a page using the system's CSS
  // classes renders identically to one using its React components. Three of
  // twenty admin routes were actually being measured, so the claim was
  // untested either way. These are the other seventeen.
  { url: '/admin/audit', auth: true },
  { url: '/admin/contract-gaps', auth: true },
  { url: '/admin/orgs', auth: true },
  { url: '/admin/parser-health', auth: true },
  { url: '/admin/parser-inference-rules', auth: true },
  { url: '/admin/parser-inference-rules/new', auth: true },
  { url: '/admin/parser-rules', auth: true },
  { url: '/admin/parser-submissions', auth: true },
  { url: '/admin/reference', auth: true },
  // NOT listed: /admin/appearance, /admin/ship-matrix and /admin/smtp. Each
  // redirects to /admin/settings, so measuring them measures that page three
  // more times — they reported an identical 165 elements, which is what gave
  // them away.
  { url: '/admin/sharing', auth: true },
  { url: '/admin/sharing/audit', auth: true },
  { url: '/admin/sharing/reports', auth: true },
  { url: '/admin/submissions', auth: true },
  { url: '/admin/waitlist', auth: true },
];

/**
 * Enough for the admin console to RENDER.
 *
 * Thirteen admin routes were reaching the error boundary on a missing fixture,
 * and the boundary is itself drawn in the idiom — so the sweep measured an
 * error page and reported the surface clean. An empty list is a legitimate
 * state for every one of these, and it is the state that exercises the most
 * chrome per route; the keys are supersets because the goal here is a rendered
 * page to measure, not a faithful payload.
 */
const EMPTY_LIST = {
  status: 200,
  body: {
    items: [],
    rules: [],
    orgs: [],
    categories: [],
    submissions: [],
    entries: [],
    reports: [],
    queue: [],
    users: [],
    total: 0,
    next_cursor: null,
    // Route-specific keys these pages destructure directly and then call
    // .filter / .map / .toLocaleString on. A generic empty list is not enough:
    // an absent key reaches the boundary as
    // `Cannot read properties of undefined`, which the sweep would report as
    // "did not render" rather than as the missing fixture it is.
    gaps: [],
    total_unmatched_runs: 0,
    findings: [],
    last_run: null,
    event_types: [],
  },
};

const ADMIN_FIXTURES = {
  'GET /v1/admin/orgs': EMPTY_LIST,
  'GET /v1/admin/parser-rules': EMPTY_LIST,
  'GET /v1/admin/parser-submissions': EMPTY_LIST,
  'GET /v1/admin/reference/categories': EMPTY_LIST,
  'GET /v1/admin/audit': EMPTY_LIST,
  'GET /v1/admin/sharing/reports': EMPTY_LIST,
  'GET /v1/admin/submissions/queue': EMPTY_LIST,
  'GET /v1/admin/users': EMPTY_LIST,
  'GET /v1/admin/contracts/gaps': EMPTY_LIST,
  'GET /v1/admin/parser-health': EMPTY_LIST,
  'GET /v1/admin/parser-inference-rules': EMPTY_LIST,
  'GET /v1/admin/event-types': EMPTY_LIST,
};

async function sweep(
  page: import('@playwright/test').Page,
  request: import('@playwright/test').APIRequestContext,
  routes: ReadonlyArray<{ url: string; auth: boolean }>,
) {
  await resetScenario(request);
  await setScenario(request, scenarioFor('idiom', ADMIN_FIXTURES));
  await loginAs(page, { handle: 'TestPilot', staffRoles: ['admin'] });

  const failures: string[] = [];
  for (const r of routes) {
    // A generous goto budget, for a reason specific to these sweeps: this one
    // test visits every surface in the app, so most of its navigations are to
    // a route no other test has compiled yet. The config's 10s navigation
    // budget is sized for a warm route, and under full-suite parallelism a
    // cold one exceeded it — the sweep then failed on the harness rather than
    // on anything it measures. Nothing about what is asserted changes.
    await page.goto(r.url, { waitUntil: 'domcontentloaded', timeout: 60_000 });
    await page.waitForLoadState('networkidle').catch(() => {});
    await page.waitForTimeout(300);
    // A route that redirects after load destroys the execution context
    // mid-measure. Settle, then retry once — and record where it actually
    // landed, because a surface that redirects was never measured and should
    // not look like a clean pass.
    const landed = new URL(page.url()).pathname;
    if (landed !== r.url) failures.push(`${r.url} — redirected to ${landed}, not measured`);
    const measure = () => page.evaluate(() => {
      const out: string[] = [];
      const root = document.querySelector('.ss-projection-root');
      if (!root) return ['no projection root'];
      // A PAGE THAT DID NOT RENDER MUST NOT PASS.
      //
      // The error boundary is itself drawn in the idiom, so a route whose data
      // fetch 599s renders a handful of compliant elements and sails through —
      // the sweep then reports the surface as clean without ever having seen
      // it. Thirteen of the twenty admin routes were doing exactly that on a
      // missing fixture, which is a green gate proving nothing.
      const text = document.body.innerText;
      if (
        text.includes('The page failed to render') ||
        text.includes('no_mock_fixture')
      ) {
        return ['did not render (error boundary) — the sweep saw nothing'];
      }
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
    let bad: string[];
    try {
      bad = await measure();
    } catch {
      await page.waitForTimeout(500);
      bad = await measure();
    }
    for (const b of bad) failures.push(`${r.url} — ${b}`);
  }
  return failures;
}

test('nothing inside a projection is a filled or rounded box', async ({ page, request }) => {
  // Sized rather than tripled, for the same reason as the admin sweep below:
  // the two together compile 31 routes in one worker, and the goto budget was
  // being exceeded on cold ones.
  test.setTimeout(300_000);
  const failures = await sweep(page, request, CORE_ROUTES);
  expect(failures, failures.join(String.fromCharCode(10))).toEqual([]);
});

test('the admin console draws in the same idiom', async ({ page, request }) => {
  // Not `test.slow()`: that triples the 30s base to 90s, and this sweep walks
  // twenty routes that no other test compiles, at roughly nine seconds each
  // cold. It timed out on the harness rather than on anything it measures —
  // which is the failure this file's own comments warn about. Sized to the
  // work instead.
  test.setTimeout(300_000);
  const failures = await sweep(page, request, ADMIN_ROUTES);
  expect(failures, failures.join(String.fromCharCode(10))).toEqual([]);
});
