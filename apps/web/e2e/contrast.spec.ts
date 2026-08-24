import { test, expect, type Page } from '@playwright/test';
import { loginAs, resetScenario, scenarioFor, setScenario } from './helpers/api-mock';

/**
 * Every rendered surface must be readable, on every calibration.
 *
 * `src/styles/palette-contrast.test.ts` pins the TOKENS. This pins their USE:
 * a token can be correct and still be applied over a tinted panel, or paired
 * with an opacity, or overridden inline — none of which the stylesheet shows.
 *
 * COMPOSITING IS THE WHOLE TRICK, and getting it wrong is how this measurement
 * lies. `.ss-card` is `rgba(127, 228, 255, 0.035)` — a 3.5% beam tint over the
 * void. A first pass at this harness read that as SOLID beam and reported two
 * elements at 1.00:1, "invisible text", which sent me looking for a bug in the
 * page. There was no bug; the measurement was wrong. Backgrounds are composited
 * down the ancestor chain, and `--void` is the floor.
 *
 * THRESHOLDS, by the project's agreed tiers:
 *   >= 7.0  for anything under 12px (the standard's 4.5 assumes ~16px)
 *   >= 4.5  for everything else
 *
 * Elements are skipped only when they cannot be read at all: zero-size, hidden,
 * or fully transparent text. A skip list of specific selectors would be a way
 * to make this pass without fixing anything, so there isn't one.
 */
const PROBE = `(() => {
  const f = (v) => { const s = v / 255; return s <= 0.03928 ? s / 12.92 : Math.pow((s + 0.055) / 1.055, 2.4); };
  const lum = (c) => 0.2126 * f(c[0]) + 0.7152 * f(c[1]) + 0.0722 * f(c[2]);
  const parse = (s) => {
    const m = String(s).match(/rgba?\\(([^)]+)\\)/);
    if (!m) return null;
    const p = m[1].split(',').map((x) => parseFloat(x));
    return { c: [p[0], p[1], p[2]], a: p.length >= 4 ? p[3] : 1 };
  };
  const over = (fg, fa, bg) => [0, 1, 2].map((i) => fg[i] * fa + bg[i] * (1 - fa));
  const VOID = [3, 6, 11];
  // Composite every translucent background from the root down to the element.
  const bgOf = (el) => {
    const layers = [];
    let n = el;
    while (n && n.nodeType === 1) {
      const p = parse(getComputedStyle(n).backgroundColor);
      if (p && p.a > 0) layers.push(p);
      n = n.parentElement;
    }
    let base = VOID;
    for (let i = layers.length - 1; i >= 0; i--) base = over(layers[i].c, layers[i].a, base);
    return base;
  };
  const ratio = (a, b) => {
    const la = lum(a), lb = lum(b);
    return (Math.max(la, lb) + 0.05) / (Math.min(la, lb) + 0.05);
  };
  const out = [];
  document.querySelectorAll('*').forEach((el) => {
    if (el.children.length > 0) return;
    const text = (el.textContent || '').trim();
    if (!text) return;
    const cs = getComputedStyle(el);
    if (cs.visibility === 'hidden' || cs.display === 'none') return;
    // aria-hidden content is not read by anyone — decorative glyphs, the
    // select caret, the chromatic fringe. Excluded because it is genuinely
    // not text, not to make a number go away: everything with an accessible
    // presence stays in.
    if (el.closest('[aria-hidden="true"]')) return;
    const r = el.getBoundingClientRect();
    if (r.width < 1 || r.height < 1) return;
    const fg = parse(cs.color);
    if (!fg || fg.a === 0) return;
    // Element opacity fades the text toward its own backdrop.
    let opacity = 1;
    let n = el;
    while (n && n.nodeType === 1) { opacity *= parseFloat(getComputedStyle(n).opacity); n = n.parentElement; }
    if (opacity < 0.05) return;
    const bg = bgOf(el);
    const eff = over(fg.c, fg.a * opacity, bg);
    const size = parseFloat(cs.fontSize);
    out.push({
      ratio: Math.round(ratio(eff, bg) * 100) / 100,
      size,
      need: size < 12 ? 7 : 4.5,
      sel: (el.tagName + '.' + String(el.className || '')).slice(0, 60),
      fg: cs.color,
      text: text.slice(0, 24),
    });
  });
  return out.filter((x) => x.ratio < x.need).sort((a, b) => a.ratio - b.ratio).slice(0, 12);
})()`;

/** The four calibrations the token file declares. */
const CALS = ['terra', 'stanton', 'pyro', 'nyx'] as const;

/** Routes covering every surface kind the port produced. */
const ROUTES: { url: string; auth: boolean; label: string }[] = [
  { url: '/', auth: false, label: 'landing (brand surface)' },
  { url: '/features', auth: false, label: 'marketing' },
  { url: '/docs', auth: false, label: 'docs prose' },
  { url: '/terms', auth: false, label: 'legal' },
  { url: '/auth/login', auth: false, label: 'auth' },
  { url: '/kb', auth: false, label: 'catalogue landing' },
  { url: '/kb/vehicle', auth: false, label: 'catalogue browse' },
  { url: '/discover', auth: false, label: 'directory' },
  { url: '/downloads', auth: false, label: 'emitter' },
  { url: '/me', auth: true, label: 'projection volume' },
  { url: '/me/travel', auth: true, label: 'travel' },
  { url: '/me/contracts', auth: true, label: 'contracts' },
  { url: '/me/loadout', auth: true, label: 'loadout' },
  { url: '/settings', auth: true, label: 'calibrate' },
  { url: '/sharing', auth: true, label: 'sharing' },
  { url: '/u/TestPilot', auth: true, label: 'public profile' },
];

async function worstOn(page: Page, url: string, cal: string) {
  await page.goto(url, { waitUntil: 'domcontentloaded' });
  // Some routes redirect client-side (`/uploads`, an unauthenticated `/me`).
  // Evaluating during that tears the execution context down mid-probe, which
  // reads as a harness failure rather than a contrast one.
  await page.waitForLoadState('networkidle').catch(() => {});
  await page.waitForTimeout(400);
  await page.evaluate((c) => {
    document.documentElement.setAttribute('data-cal', c);
    document.querySelectorAll('[data-cal]').forEach((el) => el.setAttribute('data-cal', c));
  }, cal);
  await page.waitForTimeout(250);
  return page.evaluate(PROBE) as Promise<
    { ratio: number; size: number; need: number; sel: string; text: string }[]
  >;
}

test.describe('contrast', () => {
  test.beforeEach(async ({ request }) => {
    await resetScenario(request);
    await setScenario(request, scenarioFor('contrast'));
  });

  for (const cal of CALS) {
    test(`every surface is readable on the ${cal} calibration`, async ({ page }) => {
      test.slow();
      await loginAs(page, { handle: 'TestPilot' });
      const failures: string[] = [];
      for (const r of ROUTES) {
        const bad = await worstOn(page, r.url, cal);
        for (const b of bad) {
          failures.push(
            `${cal} ${r.url} — ${b.ratio}:1 (needs ${b.need}) ${b.size}px "${b.text}" ${b.sel}`,
          );
        }
      }
      expect(failures, failures.join('\n')).toEqual([]);
    });
  }
});
