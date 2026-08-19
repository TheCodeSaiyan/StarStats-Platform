import React from 'react';
import { describe, it, expect } from 'vitest';
import { render, screen } from '@testing-library/react';
import { StatBar } from './StatBar';

describe('StatBar', () => {
  it('renders label, value, and band when present', () => {
    render(
      <StatBar
        row={{
          label: 'SCM speed',
          valueText: '262 m/s',
          fillPct: 80,
          medianPct: 30,
          band: { text: 'top 10%', tone: 'high' },
        }}
      />,
    );
    expect(screen.getByText('SCM speed')).toBeTruthy();
    expect(screen.getByText('262 m/s')).toBeTruthy();
    expect(screen.getByText('top 10%')).toBeTruthy();
  });

  it('renders a context-free row (no track) when fillPct is undefined', () => {
    render(<StatBar row={{ label: 'Hull HP', valueText: '11,900 hp' }} />);
    expect(screen.getByText('Hull HP')).toBeTruthy();
    expect(screen.queryByText('top 10%')).toBeNull();
  });
});
