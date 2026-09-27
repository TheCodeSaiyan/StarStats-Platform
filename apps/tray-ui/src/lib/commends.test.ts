import { describe, expect, it } from 'vitest';
import type { CrewOverview } from '../api';
import { commendLabel, uncommended } from './commends';

const window = (mates: Array<string | null>) => ({
  post_id: 'p',
  activity: 'mining',
  ended_at: '2026-09-28T10:00:00Z',
  closes_at: '2026-09-30T10:00:00Z',
  crew: mates.map((my_commend, i) => ({
    handle: `m${i}`,
    my_commend: my_commend as CrewOverview['windows'][number]['crew'][number]['my_commend'],
  })),
});

describe('commends', () => {
  it('labels each word, and an unknown one as itself', () => {
    expect(commendLabel('good_comms')).toBe('Good comms');
    expect(commendLabel('future_word')).toBe('future word');
  });

  it('counts crewmates not yet commended in open windows', () => {
    expect(uncommended(null)).toBe(0);
    expect(
      uncommended({
        windows: [window([null, 'reliable']), window([null, null])],
        history: [],
      }),
    ).toBe(3);
  });
});
