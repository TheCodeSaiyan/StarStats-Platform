import React from 'react';
import { describe, it, expect } from 'vitest';
import { render, screen } from '@testing-library/react';
import { ComparisonLeaderboard } from './ComparisonLeaderboard';

describe('ComparisonLeaderboard', () => {
  it('renders a card per superlative with value + winner', () => {
    render(<ComparisonLeaderboard cards={[
      { key: 'speed.scm', label: 'Fastest (SCM)', valueText: '262 m/s', winnerName: 'Avenger' },
      { key: 'health', label: 'Toughest hull', valueText: '25,013 hp', winnerName: 'Sabre' },
    ]} />);
    expect(screen.getByText('Fastest (SCM)')).toBeTruthy();
    expect(screen.getByText('262 m/s')).toBeTruthy();
    expect(screen.getByText('Avenger')).toBeTruthy();
    expect(screen.getByText('Sabre')).toBeTruthy();
  });

  it('renders nothing when there are no cards', () => {
    const { container } = render(<ComparisonLeaderboard cards={[]} />);
    expect(container.firstChild).toBeNull();
  });
});
