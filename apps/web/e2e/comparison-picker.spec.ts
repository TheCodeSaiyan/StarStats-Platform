import { expect, test } from '@playwright/test';
import { kbDetail, setScenario } from './helpers/api-mock';

/**
 * The comparison tray's "Add ship…" list used to be an absolutely
 * positioned child of its card. `.ss-card:hover` lifts the card with a
 * `transform`, which creates a stacking context, so the list's z-index
 * could not escape and the NEXT card painted over it — exactly while the
 * pointer was on the search box. A visibility assertion passes on that
 * broken layout; jsdom has no layout at all. This spec asserts what
 * actually differs: the list is not clipped by any ancestor, sits inside
 * the viewport, and is the element on top at its own centre.
 */
const DETAIL = kbDetail({
  category: 'vehicle',
  class_name: 'AEGS_Avenger_Stalker',
  display_name: 'Aegis Avenger Stalker',
  slug: 'aegis-avenger-stalker',
  summary: { manufacturer: 'Aegis Dynamics', role: 'Fighter', hull_size: 'Small' },
  metadata: {
    manufacturer: { name: 'Aegis Dynamics', code: 'AEGS' },
    speed: { scm: 210 },
    health: 1000,
  },
});

async function openPicker(page: import('@playwright/test').Page, query: string) {
  const box = page.getByRole('combobox', { name: 'Add vehicle to comparison' });
  await expect(box).toBeVisible();
  // Hover first so the card's :hover transform (the stacking-context
  // trap) is in effect while the list is open.
  await box.hover();
  await box.fill(query);
  const list = page.getByRole('listbox', { name: 'Add vehicle to comparison' });
  await expect(list).toBeVisible();
  return list;
}

test('comparison picker is neither clipped nor painted over', async ({ page, request }) => {
  await setScenario(request, {
    __id: 'comparison_picker_unclipped',
    routes: { 'GET /v1/reference/vehicle/slug/aegis-avenger-stalker': DETAIL },
  });
  await page.goto('/kb/vehicle/aegis-avenger-stalker');

  const list = await openPicker(page, 'glad');
  await expect(list.getByRole('option', { name: /Gladius/ }).first()).toBeVisible();

  const report = await page.evaluate(() => {
    const el = document.querySelector('[role="listbox"]') as HTMLElement;
    const r = el.getBoundingClientRect();
    const clippers: string[] = [];
    let a = el.parentElement;
    while (a && a !== document.documentElement) {
      const s = getComputedStyle(a);
      if (s.overflow !== 'visible' || s.overflowX !== 'visible' || s.overflowY !== 'visible') {
        const ar = a.getBoundingClientRect();
        if (r.top < ar.top || r.bottom > ar.bottom || r.left < ar.left || r.right > ar.right) {
          clippers.push(`${a.tagName}.${(a.className || '').toString().slice(0, 30)}`);
        }
      }
      a = a.parentElement;
    }
    const first = el.querySelector('[role="option"]') as HTMLElement;
    const fr = first.getBoundingClientRect();
    const hit = document.elementFromPoint(fr.left + fr.width / 2, fr.top + fr.height / 2);
    return {
      clippers,
      inViewport:
        r.top >= 0 &&
        r.left >= 0 &&
        r.bottom <= document.documentElement.clientHeight &&
        r.right <= document.documentElement.clientWidth,
      onTop: !!hit && el.contains(hit),
    };
  });

  expect(report.clippers).toEqual([]);
  expect(report.inViewport).toBe(true);
  expect(report.onTop).toBe(true);
});

test('comparison picker is fuzzy and adds on Enter', async ({ page, request }) => {
  await setScenario(request, {
    __id: 'comparison_picker_fuzzy',
    routes: { 'GET /v1/reference/vehicle/slug/aegis-avenger-stalker': DETAIL },
  });
  await page.goto('/kb/vehicle/aegis-avenger-stalker');

  // A typo: one edit away from "gladius".
  const list = await openPicker(page, 'gldius');
  await expect(list.getByRole('option').first()).toContainText(/Gladius/);
  await page.keyboard.press('Enter');
  await expect(list).toHaveCount(0);
  // The pick landed as a chip in the tray.
  await expect(page.getByRole('button', { name: /Remove .*Gladius/ })).toBeVisible();
});
