import { describe, expect, it } from 'vitest';
import { activityLabel, crewHandles, matchSystem } from './lfg';

describe('matchSystem', () => {
  const systems = ['Nyx', 'Pyro', 'Stanton'];
  it('uses the server spelling', () => {
    expect(matchSystem(systems, 'stanton')).toBe('Stanton');
    expect(matchSystem(systems, ' PYRO ')).toBe('Pyro');
  });
  it('pre-fills nothing the server would refuse', () => {
    expect(matchSystem(systems, 'Castra')).toBe('');
    expect(matchSystem(systems, null)).toBe('');
  });
});

describe('crewHandles', () => {
  it('lists accepted crew only, one per line', () => {
    expect(
      crewHandles([
        { handle: 'Alice', status: 'accepted' },
        { handle: 'Bob', status: 'requested' },
        { handle: 'Carol', status: 'accepted' },
      ]),
    ).toBe('Alice\nCarol');
  });
});

describe('labels', () => {
  it('fall back to a readable id', () => {
    expect(activityLabel('bounty_hunting')).toBe('Bounty hunting');
    expect(activityLabel('base_building')).toBe('Base building');
  });
});
