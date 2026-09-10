import { describe, it, expect } from 'vitest';
import { describePublicScope, summarisePublicScope } from './public-scope';
import type { ShareScope } from './api';

const scope = (over: Partial<ShareScope> = {}): ShareScope =>
  ({ kind: 'full', ...over }) as ShareScope;

describe('describePublicScope', () => {
  it('names the clamp when one is set', () => {
    const d = describePublicScope(
      scope({ max_event_types: 5, window_days: 30 }),
    );
    expect(d.uncapped).toBe(false);
    expect(d.published).toContain('Your 5 busiest event types, with counts');
    expect(d.published).toContain(
      'An activity heatmap covering the last 30 days',
    );
  });

  // The state this whole change exists to make visible: a profile made
  // public before the clamp existed publishes the complete histogram.
  // The owner cannot decide whether that is what they wanted unless the
  // page says so in those words.
  it('says plainly when nothing is clamping the profile', () => {
    const d = describePublicScope(null);
    expect(d.uncapped).toBe(true);
    expect(d.published).toContain(
      'Every event type you have logged, with counts',
    );
    expect(d.published).toContain(
      'An activity heatmap covering the last 90 days',
    );
  });

  // A scope that only carries a widget list clamps nothing about the
  // payload, so calling it "capped" would be a comforting lie.
  it('does not count a widget-only scope as capped', () => {
    const d = describePublicScope(scope({ deny_widgets: ['economy'] }));
    expect(d.uncapped).toBe(true);
  });

  it('treats a window without a type cap as still capped', () => {
    const d = describePublicScope(scope({ window_days: 7 }));
    expect(d.uncapped).toBe(false);
    expect(d.published).toContain(
      'Every event type you have logged, with counts',
    );
  });
});

describe('summarisePublicScope', () => {
  it('leads with the histogram, which is the field that surprises people', () => {
    expect(summarisePublicScope(scope({ max_event_types: 5, window_days: 30 })))
      .toBe('Top 5 event types · 30 days');
  });

  it('is explicit about an unclamped profile', () => {
    expect(summarisePublicScope(null)).toBe('All event types · 90 days');
  });
});
