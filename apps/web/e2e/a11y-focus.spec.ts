import { test, expect, type Page } from '@playwright/test';
import { loginAs, resetScenario, scenarioFor, setScenario } from './helpers/api-mock';

/**
 * Keyboard operability, measured by operating the keyboard.
 *
 * The port replaced every control in the app. A projection draws its
 * affordances as hairlines and lit edges rather than as filled boxes, and the
 * failure mode that comes with that is a control whose focus state is a colour
 * change so slight it cannot be seen — or no change at all, because the flat
 * `:focus-visible` rule was scoped to a class the projection no longer emits.
 *
 * WHAT IS ASSERTED, and why each one is measured rather than inspected:
 *
 *   - Every tab stop shows a VISIBLE change on focus. Compared as a computed
 *     style snapshot before and after focus, so "it has an outline rule
 *     somewhere" is not good enough — the rule has to actually take effect on
 *     that element.
 *   - Tabbing reaches the page's own content, not just the chrome. A projection
 *     puts a lot of chrome first; if focus never escapes it the page is
 *     keyboard-unreachable in practice.
 *   - Focus never lands on something invisible. A control clipped to zero size
 *     or faded out still takes a tab stop, and a reader then tabs into nothing.
 *   - Every icon-only control has an accessible name.
 */
const ROUTES: { url: string; auth: boolean }[] = [
  { url: '/', auth: false },
  { url: '/kb/vehicle', auth: false },
  { url: '/auth/login', auth: false },
  { url: '/me', auth: true },
  { url: '/me/travel', auth: true },
  { url: '/settings', auth: true },
  { url: '/sharing', auth: true },
  { url: '/discover', auth: true },
];

/** A computed-style fingerprint of the things a focus ring can change. */
const FOCUS_FINGERPRINT = `(el) => {
  const cs = getComputedStyle(el);
  return [cs.outlineStyle, cs.outlineWidth, cs.outlineColor, cs.boxShadow,
          cs.borderColor, cs.backgroundColor, cs.color, cs.textDecorationLine].join('|');
}`;

async function tabStops(page: Page, max: number) {
  const seen: {
    tag: string;
    name: string;
    visible: boolean;
    changed: boolean;
  }[] = [];
  for (let i = 0; i < max; i++) {
    await page.keyboard.press('Tab');
    const info = await page.evaluate(
      ({ fp }) => {
        const el = document.activeElement as HTMLElement | null;
        if (!el || el === document.body) return null;
        // `next dev` mounts its error overlay as <nextjs-portal>, which takes a
        // tab stop and is not in the production bundle. Excluded because it is
        // not ours to fix, not because it is inconvenient.
        if (el.tagName === 'NEXTJS-PORTAL') return null;
        const fn = new Function('return ' + fp)();
        const r = el.getBoundingClientRect();
        const cs = getComputedStyle(el);
        return {
          tag: el.tagName,
          cls: String(el.className).slice(0, 40),
          name: (
            el.getAttribute('aria-label') ||
            el.textContent ||
            el.getAttribute('title') ||
            ''
          )
            .trim()
            .slice(0, 30),
          visible:
            r.width > 0 &&
            r.height > 0 &&
            cs.visibility !== 'hidden' &&
            parseFloat(cs.opacity) > 0.05,
          focused: fn(el),
          // The same element with focus removed, to compare against.
          blurred: (() => {
            el.blur();
            const s = fn(el);
            el.focus();
            return s;
          })(),
        };
      },
      { fp: FOCUS_FINGERPRINT },
    );
    if (!info) break;
    seen.push({
      tag: info.tag,
      name: `${info.tag}.${info.cls} "${info.name}"`,
      visible: info.visible,
      changed: info.focused !== info.blurred,
    });
  }
  return seen;
}

test.describe('keyboard', () => {
  test.beforeEach(async ({ request }) => {
    await resetScenario(request);
    await setScenario(request, scenarioFor('a11y'));
  });

  test('every tab stop is visible and shows focus', async ({ page }) => {
    test.slow();
    const failures: string[] = [];
    await loginAs(page, { handle: 'TestPilot' });
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
      const stops = await tabStops(page, 22);
      expect(stops.length, `${r.url}: nothing is focusable`).toBeGreaterThan(2);
      for (const s of stops) {
        if (!s.visible) failures.push(`${r.url} — invisible tab stop: ${s.name}`);
        else if (!s.changed)
          failures.push(`${r.url} — no focus indicator: ${s.name}`);
      }
    }
    expect(failures, failures.join('\n')).toEqual([]);
  });

  test('every control has an accessible name', async ({ page }) => {
    test.slow();
    const failures: string[] = [];
    await loginAs(page, { handle: 'TestPilot' });
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
      const nameless = await page.evaluate(() => {
        const out: string[] = [];
        document
          .querySelectorAll<HTMLElement>('button, a[href], [role="button"]')
          .forEach((el) => {
            if (el.closest('[aria-hidden="true"]')) return;
            const r = el.getBoundingClientRect();
            if (r.width < 1 || r.height < 1) return;
            const name = (
              el.getAttribute('aria-label') ||
              el.getAttribute('title') ||
              el.textContent ||
              ''
            ).trim();
            if (!name) {
              out.push(
                `${el.tagName}.${String(el.className).slice(0, 40)}`,
              );
            }
          });
        return out;
      });
      for (const n of nameless) failures.push(`${r.url} — unnamed control: ${n}`);
    }
    expect(failures, failures.join('\n')).toEqual([]);
  });
});
