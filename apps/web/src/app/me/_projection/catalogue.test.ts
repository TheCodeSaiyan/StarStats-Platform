import { describe, expect, it } from 'vitest';
import { CALLOUT_SLOTS } from 'holo';
import { PROJECTION_CATALOGUE } from './catalogue';

/**
 * The field has to be able to draw everything the catalogue offers.
 *
 * It could not. The catalogue lists seven callouts and the field had six
 * slots, so a reader who enabled every one was told "+1 undrawn · reorder"
 * for the rest of the session — true about capacity, unactionable in fact,
 * since reordering only changes WHICH six appear. There was no arrangement
 * that cleared the message, which is what made it read as a fault.
 *
 * The two numbers lived apart — an array in the design system and a literal
 * `max = 6` beside it — so nothing noticed when the catalogue grew. This is
 * the check that would have.
 */
describe('projection callout capacity', () => {
  const callouts = PROJECTION_CATALOGUE.filter((e) => e.group === 'Callouts');

  it('has a slot for every callout the reader can enable', () => {
    expect(callouts.length).toBeGreaterThan(0);
    expect(CALLOUT_SLOTS.length).toBeGreaterThanOrEqual(callouts.length);
  });

  it('never overflows when everything is switched on', () => {
    // What CalloutField computes: all - min(all, max).
    const undrawn = Math.max(0, callouts.length - CALLOUT_SLOTS.length);
    expect(undrawn).toBe(0);
  });

  it('names a group after something the reader can point at', () => {
    // "Ring" did not say WHICH ring, and a reader asked whether it meant the
    // one in the middle of the screen. It does.
    const groups = new Set(PROJECTION_CATALOGUE.map((e) => e.group));
    expect(groups.has('Centre ring')).toBe(true);
    expect(groups.has('Ring' as never)).toBe(false);
  });
});
