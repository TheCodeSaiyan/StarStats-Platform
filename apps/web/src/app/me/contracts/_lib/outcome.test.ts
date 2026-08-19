import { describe, it, expect } from 'vitest';
import {
  outcomeText,
  outcomeBadgeVariant,
  stepStateLabel,
  runDurationSecs,
  byAcceptedDesc,
} from './outcome';

/**
 * `outcome.ts` was extracted from the page specifically so this mapping
 * could be exercised directly. Going through the page only ever covered
 * the two `closed_by` values its fixtures happened to use, which left the
 * inference-vs-observation distinction — the whole point of the module —
 * almost entirely untested.
 *
 * The `CLOSED_BY` table below is the complete set the server can emit;
 * it is a transcription of `closed_by_str` in `crates/starstats-server/
 * src/repo.rs`. If a `ClosedBy` variant is ever added, this table goes
 * stale silently — the guard against that is the exhaustive Rust `match`
 * in `closed_by_str` itself, which will not compile until the new variant
 * is handled there.
 */

/** [closed_by, expected text, expected badge variant] */
const CLOSED_BY: [string, string, string][] = [
  ['hud_complete', 'completed', 'ok'],
  ['hud_failed', 'failed', 'danger'],
  ['hud_withdrawn', 'withdrawn', ''],
  ['session_end', 'abandoned — app exit', 'warn'],
  ['game_crash', 'abandoned — game crash', 'warn'],
  ['session_gap', 'abandoned — session gap', 'warn'],
  ['shard_change', 'abandoned — changed server', 'warn'],
  ['superseded', 'superseded by a later accept', ''],
];

describe('outcomeText', () => {
  it.each(CLOSED_BY)('maps closed_by=%s to human text', (closedBy, text) => {
    // `state` is deliberately a value that would produce different text if
    // it were being consulted — proving `closed_by` alone decides these.
    expect(outcomeText('unknown', closedBy)).toBe(text);
    expect(outcomeText('in_progress', closedBy)).toBe(text);
  });

  it('never returns a raw enum value for any emittable closed_by', () => {
    for (const [closedBy] of [...CLOSED_BY, ['none']]) {
      expect(outcomeText('unknown', closedBy)).not.toContain('_');
      expect(outcomeText('in_progress', closedBy)).not.toContain('_');
    }
  });

  // The `none` tiebreak: `closed_by` carries no information, so `state` is
  // the only thing separating "this contract is still running" from "the
  // stream ended and we never saw it close". Collapsing these would report
  // live contracts as unresolved.
  it('distinguishes still-running from no-evidence when closed_by is none', () => {
    expect(outcomeText('in_progress', 'none')).toBe('still in progress');
    expect(outcomeText('unknown', 'none')).toBe('no outcome recorded');
    expect(outcomeText('abandoned', 'none')).toBe('no outcome recorded');
  });
});

describe('outcomeBadgeVariant', () => {
  it.each(CLOSED_BY)('gives closed_by=%s the %s variant', (closedBy, _text, variant) => {
    expect(outcomeBadgeVariant(closedBy)).toBe(variant);
  });

  // Inferred closes are a best guess, not eyewitness. They must not wear
  // the same definite colour as an outcome the HUD actually reported.
  it('never paints an inferred close with an observed close\'s colour', () => {
    for (const inferred of ['session_end', 'game_crash', 'session_gap', 'shard_change']) {
      expect(outcomeBadgeVariant(inferred)).toBe('warn');
    }
    expect(outcomeBadgeVariant('hud_complete')).toBe('ok');
    expect(outcomeBadgeVariant('none')).toBe('');
  });
});

describe('stepStateLabel', () => {
  it.each([
    ['in_progress', 'in progress'],
    ['complete', 'complete'],
    ['withdrawn', 'withdrawn'],
    ['failed', 'failed'],
  ])('maps step state %s', (state, label) => {
    expect(stepStateLabel(state)).toBe(label);
  });

  it('de-snakes an unrecognized state rather than leaking it raw', () => {
    expect(stepStateLabel('some_future_state')).toBe('some future state');
  });
});

describe('runDurationSecs', () => {
  it('returns the span in seconds', () => {
    expect(runDurationSecs('2026-07-20T10:00:00Z', '2026-07-20T10:30:00Z')).toBe(1800);
  });

  // Each guard below exists to keep a bogus duration off the page. Nothing
  // else in the suite fires them — every page fixture carries a valid pair.
  it.each([
    ['missing start', null, '2026-07-20T10:30:00Z'],
    ['missing end', '2026-07-20T10:00:00Z', null],
    ['both missing', null, null],
    ['unparsable start', 'not-a-date', '2026-07-20T10:30:00Z'],
    ['unparsable end', '2026-07-20T10:00:00Z', 'not-a-date'],
    // A run that closed "before" it was accepted would otherwise render a
    // negative duration, which reads as a data-integrity bug to the user.
    ['inverted range', '2026-07-20T10:30:00Z', '2026-07-20T10:00:00Z'],
  ])('returns null on %s', (_case, start, end) => {
    expect(runDurationSecs(start, end)).toBeNull();
  });

  it('allows a zero-length run', () => {
    expect(runDurationSecs('2026-07-20T10:00:00Z', '2026-07-20T10:00:00Z')).toBe(0);
  });
});

describe('byAcceptedDesc', () => {
  it('sorts newest first and puts runs with no accepted_at last', () => {
    const sorted = [
      { accepted_at: '2026-07-20T10:00:00Z' },
      { accepted_at: null },
      { accepted_at: '2026-07-22T10:00:00Z' },
      { accepted_at: '2026-07-21T10:00:00Z' },
    ].sort(byAcceptedDesc);

    expect(sorted.map((r) => r.accepted_at)).toEqual([
      '2026-07-22T10:00:00Z',
      '2026-07-21T10:00:00Z',
      '2026-07-20T10:00:00Z',
      null,
    ]);
  });
});
