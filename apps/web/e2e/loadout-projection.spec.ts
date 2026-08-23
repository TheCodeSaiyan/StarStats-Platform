/**
 * The loadout surface, in the projection.
 *
 * NOT a capture spec any more. This file began as scaffolding for the port —
 * a set of `goto` + `waitForTimeout` + `screenshot` cases whose only job was
 * producing images to judge, plus the fixtures they needed. Those 28 cases
 * asserted nothing, slept for half a second each, and are gone; what is left
 * are the assertions written alongside them, which are about behaviour and
 * outlive the port.
 */
import { test, expect } from '@playwright/test';
import { loginAs, resetScenario, scenarioFor, setScenario } from './helpers/api-mock';

const consoleErrors: string[] = [];

const FIXTURES = {
  'GET /v1/me/events': {
    status: 200,
    body: {
      events: [
        {
          seq: 91422,
          event_type: 'burst_summary',
          event_timestamp: '2026-08-21T20:14:03Z',
          hidden_at: null,
          log_source: 'game.log',
          source_offset: 5540123,
          resolved_location: null,
          payload: {
            kind: 'loadout_restore',
            items: [
              { class: 'grin_helmet_01', port: 'armor_helmet', category: 'Char_Armor_Helmet' },
              { class: 'grin_core_01', port: 'armor_torso', category: 'Char_Armor_Torso' },
              { class: 'grin_arms_01', port: 'armor_arms', category: 'Char_Armor_Arms' },
              { class: 'grin_legs_01', port: 'armor_legs', category: 'Char_Armor_Legs' },
              { class: 'behr_rifle_01', port: 'weapon_body_primary', category: 'Weapon_FPS_Rifle' },
              { class: 'behr_pistol_01', port: 'weapon_body_secondary', category: 'Weapon_FPS_Pistol' },
              { class: 'mag_rifle_01', port: 'magazine_01', category: 'Magazine' },
              // Excluded by port — anatomy, never gear.
              { class: 'human_eye_l', port: 'eyes_ItemPort', category: 'Char_Anatomy' },
            ],
          },
        },
      ],
    },
  },
  'POST /v1/reference/resolve': {
    status: 200,
    body: {
      resolved: {
        grin_helmet_01: { display_name: 'GRIN Ballistic Helmet', slug: 'grin-helmet', category: 'item', classification: 'FPS.Armor.Helmet', classification_label: 'Helmet', has_image: false },
        grin_core_01: { display_name: 'GRIN Ballistic Core', slug: 'grin-core', category: 'item', classification: 'FPS.Armor.Torso', classification_label: 'Torso', has_image: false },
        grin_arms_01: { display_name: 'GRIN Ballistic Arms', slug: 'grin-arms', category: 'item', classification: 'FPS.Armor.Arms', classification_label: 'Arms', has_image: false },
        grin_legs_01: { display_name: 'GRIN Ballistic Legs', slug: 'grin-legs', category: 'item', classification: 'FPS.Armor.Legs', classification_label: 'Legs', has_image: false },
        behr_rifle_01: { display_name: 'BEHR P8-AR', slug: 'behr-p8-ar', category: 'weapon', classification: 'FPS.Weapon.Rifle', classification_label: 'Rifle', has_image: false },
        behr_pistol_01: { display_name: 'BEHR S-38', slug: 'behr-s38', category: 'weapon', classification: 'FPS.Weapon.Pistol', classification_label: 'Pistol', has_image: false },
        mag_rifle_01: { display_name: 'P8-AR Magazine', slug: 'p8-ar-mag', category: 'item', classification: 'FPS.WeaponAttachment.Magazine', classification_label: 'Magazine', has_image: false },
      },
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
  await setScenario(request, scenarioFor('loadout-projection', FIXTURES));
  await loginAs(page, { handle: 'StarStatsDemo' });
  await page.setViewportSize({ width: 1440, height: 900 });
});

test('armour reaches its body slot; an empty slot still names itself', async ({
  page,
}) => {
  await page.goto('/me/loadout');
  await expect(page.locator('.hp-slot--head')).toContainText(
    'GRIN Ballistic Helmet',
  );
  // Nothing was restored to the back or undersuit, and the GAP is the
  // information — so the slot is still drawn and still labelled.
  await expect(page.locator('.hp-slot--back.hp-slot--empty')).toBeVisible();
  await expect(page.locator('.hp-slot--undersuit.hp-slot--empty')).toBeVisible();
});

test('anatomy is filtered out by port, never shown as gear', async ({
  page,
}) => {
  // `isExcludedPort` drops `*_ItemPort` anatomy. Surfacing an eyeball as
  // carried equipment would be nonsense the log never claimed.
  await page.goto('/me/loadout');
  await expect(page.locator('.hp-paperdoll')).toBeVisible();
  await expect(page.getByText(/human_eye/i)).toHaveCount(0);
});

test('gear is grouped, and empty groups are not drawn', async ({ page }) => {
  // Classifications are `FPS.`-prefixed (`FPS.WeaponAttachment.Magazine`) —
  // read `groupForItem`, do not guess. An unprefixed value silently falls
  // through to "other", which looks like a grouping bug and is not one.
  await page.goto('/me/loadout');
  await expect(page.locator('.hp-plane', { hasText: 'Weapons' })).toBeVisible();
  await expect(page.locator('.hp-plane', { hasText: 'Magazines' })).toBeVisible();
  // Nothing was throwable, so the heading is absent rather than empty.
  await expect(page.locator('.hp-plane', { hasText: 'Throwables' })).toHaveCount(0);
});

test('the rail is hidden when there is only one group', async ({ page }) => {
  // The paperdoll and the carried gear are one view of one kit; a one-item
  // rail would read as a control that does not work.
  await page.goto('/me/loadout');
  await expect(page.locator('.hp-paperdoll')).toBeVisible();
  await expect(page.locator('.hp-lens')).toHaveCount(0);
});

test('the page has exactly one h1, naming the page', async ({ page }) => {
  await page.goto('/me/loadout');
  await expect(page.locator('h1')).toHaveCount(1);
  await expect(page.locator('h1')).toHaveText('Loadout');
});

test('no console errors', async ({ page }) => {
  await page.goto('/me/loadout');
  await expect(page.locator('.hp-paperdoll')).toBeVisible();
  await page.waitForTimeout(1200);
  if (consoleErrors.length) {
    console.log(`CONSOLE ERRORS:\n${consoleErrors.join('\n---\n')}`);
  }
  expect(consoleErrors).toEqual([]);
});
