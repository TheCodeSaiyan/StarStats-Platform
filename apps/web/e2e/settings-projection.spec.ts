/**
 * The settings surface, in the projection.
 *
 * NOT a capture spec any more. This file began as scaffolding for the port —
 * a set of `goto` + `waitForTimeout` + `screenshot` cases whose only job was
 * producing images to judge, plus the fixtures they needed. Those 28 cases
 * asserted nothing, slept for half a second each, and are gone; what is left
 * are the assertions written alongside them, which are about behaviour and
 * outlive the port.
 */
import { readFileSync } from 'node:fs';
import { test, expect, type Page } from '@playwright/test';
import {
  currentUser,
  loginAs,
  resetScenario,
  scenarioFor,
  setScenario,
} from './helpers/api-mock';
import { liveIn, liveStage } from './helpers/shell';


const consoleErrors: string[] = [];

/** An account with 2FA off and an unverified RSI handle — the state with the
 *  most on screen (the ownership procedure and the 2FA explainer both show). */
const FIXTURES = {
  // An UNVERIFIED handle, so the ownership procedure renders rather than the
  // verified branch.
  'GET /v1/auth/me': {
    status: 200,
    body: { ...currentUser.body, rsi_verified: false, totp_enabled: false },
  },
  'GET /v1/me/preferences': {
    status: 200,
    body: { theme: 'terra', theme_wave_speed: 'normal', timezone: null },
  },
  'POST /v1/auth/rsi/start': {
    status: 200,
    body: {
      code: 'SS-VERIFY-7Q4K2M9X',
      expires_at: '2026-08-23T12:00:00Z',
    },
  },
};

test.beforeEach(async ({ page, request }) => {
  consoleErrors.length = 0;
  page.on('console', (m) => {
    if (m.type() === 'error') consoleErrors.push(m.text());
  });
  page.on('pageerror', (e) => consoleErrors.push(`pageerror: ${e.message}`));
  await resetScenario(request);
  await setScenario(request, scenarioFor('settings-projection', FIXTURES));
  await loginAs(page, { handle: 'StarStatsDemo' });
  await page.setViewportSize({ width: 1440, height: 900 });
});

async function openGroup(page: Page, name: string): Promise<void> {
  await page.locator('.hp-lens button', { hasText: name }).click();
}

test('a fragment opens its group (the lens-rail cost)', async ({ page }) => {
  // This is the behaviour grouping put at risk: every server action redirects
  // to a fragment, and the target section is not mounted until its group is.
  await page.goto('/settings#security');
  await expect(page.locator('.hp-settings')).toBeVisible();
  await expect(
    page.getByRole('heading', { name: 'Two-factor authentication' }),
  ).toBeVisible();
});

test('picking a calibration repaints the beam in place', async ({ page }) => {
  // The regression this guards: `setCalibrationAction` deliberately does not
  // revalidate, so rendering the server prop meant the pips fired the shock
  // ring and scan wipe over a volume that stayed the old colour until the next
  // navigation. Asserting `data-cal` on the STAGE (not on <html>, which the
  // projection never sets) is what actually differs.
  await page.goto('/settings');
  await expect(page.locator('.hp-settings')).toBeVisible();
  await expect(liveStage(page)).toHaveAttribute('data-cal', 'terra');

  await page.locator('.hp-calchoice button', { hasText: 'Pyro' }).click();

  await expect(liveStage(page)).toHaveAttribute('data-cal', 'pyro');
  // ...and without a navigation.
  await expect(page).toHaveURL(/\/settings$/);
});

test('the page has exactly one h1, naming the page', async ({ page }) => {
  // Every flat screen these replaced had an h1; the projection has no titled
  // surface of its own, so it went missing on four ported pages at once
  // before a test noticed. The final crumb step carries it.
  await page.goto('/settings');
  await expect(page.locator('h1')).toHaveCount(1);
  await expect(page.locator('h1')).toHaveText('Calibrate');
});

test('no console errors on settings', async ({ page }) => {
  await page.goto('/settings');
  await expect(page.locator('.hp-settings')).toBeVisible();
  // Every group, because a group change mounts a whole different subtree —
  // the 2FA pane in particular. Checking only the landing group would miss it.
  for (const g of ['Account', 'Security', 'Danger', 'General']) {
    await openGroup(page, g);
    await page.waitForTimeout(250);
  }
  await page.waitForTimeout(1200);
  if (consoleErrors.length) {
    console.log(`CONSOLE ERRORS:\n${consoleErrors.join('\n---\n')}`);
  }
  expect(consoleErrors).toEqual([]);
});

test('settings states the retention window', async ({ page }) => {
  // `Calibrate.jsx` carries a Retention pane and this page had none, so the
  // single most important fact about a reader's data — that it is bounded at a
  // year — was surfaced nowhere in settings.
  //
  // The spec's storage rows (parsed bytes, free-tier cap) are deliberately
  // absent: the product has no storage accounting, and inventing them on the
  // page where a reader goes to find out what is kept would be the worst
  // possible place for a fabricated figure. The export row it also imagined
  // is now real (`GET /v1/me/export`) and asserted separately below.
  await page.goto('/settings');
  await expect(page.locator('.hp-settings')).toBeVisible();
  const pane = page.locator('.hp-pane', { hasText: 'Retention' }).first();
  await expect(pane).toContainText('365 days');
  await expect(pane).toContainText('Deleted, not archived');
  // "All" means the retention limit, not all time — the vocabulary rule.
  await expect(pane).toContainText(/All rather than all time/i);
  // No invented figures.
  await expect(pane).not.toContainText(/free-tier|MB/i);
});

test('settings offers the manifest export in the three shipped formats', async ({ page }) => {
  await page.goto('/settings');
  const pane = page.locator('.hp-pane', { hasText: 'Retention' }).first();
  await expect(pane).toContainText('NDJSON');
  // Exactly one link per format, pointing at the streaming route handler —
  // not a server action, which cannot hand the browser a file.
  for (const format of ['ndjson', 'csv', 'zip']) {
    await expect(
      pane.locator(`a[href="/settings/export?format=${format}"]`),
    ).toHaveCount(1);
  }
});

test('export link downloads the file the API streams, name intact', async ({
  page,
  request,
}) => {
  const filename = 'starstats-export-starstatsdemo-20260907.ndjson';
  await setScenario(
    request,
    scenarioFor('settings-export', {
      ...FIXTURES,
      'GET /v1/me/export': {
        status: 200,
        rawBody: '{"kind":"export","format_version":1}\n{"kind":"account"}\n',
        headers: {
          'content-type': 'application/x-ndjson; charset=utf-8',
          'content-disposition': `attachment; filename="${filename}"`,
        },
      },
    }),
  );
  await page.goto('/settings');
  const [download] = await Promise.all([
    // Explicit timeout: `waitForEvent` otherwise inherits `actionTimeout`
    // (5s), which is the right budget for a click and the wrong one for
    // "wait while `next dev` compiles /settings/export and streams a file".
    // It timed out on a cold route in a full-suite run on 2026-09-12 and
    // passed in isolation every time, which is the shape of a budget that is
    // too tight rather than a broken download.
    page.waitForEvent('download', { timeout: 30_000 }),
    // BOTH HALVES NEED THE BUDGET, and only the wait got it in 2026-09-12.
    // The suite went red again on 2026-09-13 with `locator.click: Timeout
    // 5000ms exceeded` — the click, not the wait. A click is not instant
    // here: Playwright holds it until the anchor is actionable, and on a
    // cold run /settings is still compiling and hydrating, so the element
    // is not stable inside 5s. Same cause, same shape (passed in isolation
    // in 4.1s), one timeout short of fixed.
    page
      .locator('a[href="/settings/export?format=ndjson"]')
      .click({ timeout: 30_000 }),
  ]);
  expect(download.suggestedFilename()).toBe(filename);
  const saved = await download.path();
  const text = readFileSync(saved, 'utf8');
  expect(text).toContain('"kind":"export"');
  expect(text).toContain('"kind":"account"');
  // The page itself did not navigate away from settings.
  await expect(page).toHaveURL(/\/settings/);
});

test('export cooldown comes back as a settings notice, not a broken download', async ({
  page,
  request,
}) => {
  await setScenario(
    request,
    scenarioFor('settings-export-429', {
      ...FIXTURES,
      'GET /v1/me/export': {
        status: 429,
        body: { error: 'too_many_requests' },
      },
    }),
  );
  await page.goto('/settings');
  await page.locator('a[href="/settings/export?format=zip"]').click();
  await expect(page).toHaveURL(/\/settings\?error=export_too_soon/);
  // Filter + count, not a bare strict locator: while the redirected page
  // streams in, the loading boundary's `.hp-settings` and the arriving one
  // coexist for a moment, and a strict-mode violation is thrown rather than
  // retried — which is why this went red on slower runs (CI and a loaded
  // desktop) with the notice visibly on screen. Waiting for exactly one
  // element carrying the notice retries through the transition.
  await expect(
    page.locator('.hp-settings').filter({ hasText: /less than a minute ago/i }),
  ).toHaveCount(1);
});

test('the document sits in a reading column, not the full-width pane', async ({
  page,
}) => {
  // Same fault and same fix as `/sharing` (see e2e/sharing-layout.spec.ts).
  // Calibrate is eleven sections of prose and forms with no table in any of
  // them, so it takes the same `measure="reading"` its sibling does — its copy
  // was capped at 62ch (467px measured) inside a 1040px pane, ending every
  // paragraph at 45% of its own panel. The width is the assertion because the
  // wide render is perfectly visible.
  //
  // Appended rather than inserted: this suite runs single-worker against a
  // `next dev` server, and several specs assert on FIRST PAINT with no settle
  // (`lens-memory`, deliberately). Adding a navigation earlier in the file
  // shifts every subsequent test's timing and surfaces those latent races —
  // it did exactly that, in specs this change cannot touch.
  await page.goto('/settings');
  const inner = liveIn(page, '.hp-settings__inner');
  await expect(inner).toBeVisible();
  const box = (await inner.boundingBox())!;
  expect(box.width).toBeLessThanOrEqual(860);
});

test('recalibration runs at the wave speed the reader chose', async ({
  page,
  request,
}) => {
  /**
   * /settings has offered a wave-speed preference (off / slow / normal /
   * fast) since before the projection port, and the root layout has stamped
   * it on `<html data-wave-speed>` on every render — two fetches a page.
   * Nothing read it. The animation it governed (`applyThemeWithWave`) lost
   * its caller in the port, which replaced it with the recalibration event,
   * and that event ran at a fixed 700ms whatever the reader had chosen.
   *
   * Now `patterns-holo.css` scales the recal durations by the attribute, on
   * the wave's own table: 700ms at normal, 350 at fast, 1100 at slow, and
   * NO animation at off. This forces the event window open and reads the
   * computed style — the attribute is normally cleared by a timer, and the
   * `off` window is a single frame, so racing it would be the flaky version
   * of this test. Asserting the property that differs, not visibility: the
   * shock ring is `opacity: 0` at rest either way.
   */
  const cases = [
    ['normal', 700],
    ['fast', 350],
    ['slow', 1100],
  ] as const;

  for (const [speed, shockMs] of cases) {
    await setScenario(
      request,
      scenarioFor(`settings-wave-${speed}`, {
        ...FIXTURES,
        'GET /v1/me/preferences': {
          status: 200,
          body: { theme: 'terra', theme_wave_speed: speed, timezone: null },
        },
      }),
    );
    await loginAs(page, { handle: 'StarStatsDemo' });
    await page.goto('/settings');
    await expect(liveStage(page)).toBeVisible();

    const seen = await page.evaluate(() => {
      const stage = document.querySelector('.hp-stage:not([data-pending])')!;
      stage.setAttribute('data-recal', '');
      const cs = getComputedStyle(stage.querySelector('.hp-shock')!);
      const out = {
        stamped: document.documentElement.dataset.waveSpeed,
        name: cs.animationName,
        ms: Math.round(parseFloat(cs.animationDuration) * 1000),
      };
      stage.removeAttribute('data-recal');
      return out;
    });

    expect(seen.stamped, 'layout stamps the preference').toBe(speed);
    expect(seen.name, `${speed} animates`).toBe('hpShock');
    expect(Math.abs(seen.ms - shockMs), `${speed}: ${seen.ms}ms vs ${shockMs}ms`).toBeLessThanOrEqual(5);
  }

  await setScenario(
    request,
    scenarioFor('settings-wave-off', {
      ...FIXTURES,
      'GET /v1/me/preferences': {
        status: 200,
        body: { theme: 'terra', theme_wave_speed: 'off', timezone: null },
      },
    }),
  );
  await loginAs(page, { handle: 'StarStatsDemo' });
  await page.goto('/settings');
  await expect(liveStage(page)).toBeVisible();
  const off = await page.evaluate(() => {
    const stage = document.querySelector('.hp-stage:not([data-pending])')!;
    stage.setAttribute('data-recal', '');
    const names = ['.hp-shock', '.hp-wipe', '.hp-emit'].map(
      (sel) => getComputedStyle(stage.querySelector(sel)!).animationName,
    );
    stage.removeAttribute('data-recal');
    return { stamped: document.documentElement.dataset.waveSpeed, names };
  });
  expect(off.stamped).toBe('off');
  // "Off" means off — every animation in the event, not just the shock.
  expect(off.names, 'off draws nothing').toEqual(['none', 'none', 'none']);
});
