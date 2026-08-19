import React from 'react';
import { describe, it, expect } from 'vitest';
import { render } from '@testing-library/react';
import { ComparisonRadar } from './ComparisonRadar';

describe('ComparisonRadar', () => {
  it('renders one polygon per series + a legend entry per series', () => {
    const { container, getByText } = render(
      <ComparisonRadar
        axisLabels={['Speed', 'Hull', 'Shield']}
        series={[
          { slug: 'a', name: 'Avenger', color: '#E8A23C', values: [1, 0.3, 0.6] },
          { slug: 'b', name: 'Gladius', color: '#5BC8C0', values: [0.4, 0.1, 0.5] },
        ]}
      />,
    );
    expect(container.querySelectorAll('polygon[data-series]')).toHaveLength(2);
    expect(getByText('Avenger')).toBeTruthy();
    expect(getByText('Gladius')).toBeTruthy();
  });

  it('renders nothing for fewer than 3 axes is NOT the case here — but a 3-axis set renders', () => {
    const { container } = render(
      <ComparisonRadar
        axisLabels={['Speed', 'Hull', 'Shield']}
        series={[{ slug: 'a', name: 'A', color: '#E8A23C', values: [1, 0.5, 0.2] }]}
      />,
    );
    expect(container.querySelector('svg')).toBeTruthy();
  });

  it('renders null for fewer than 3 axes', () => {
    const { container } = render(
      <ComparisonRadar axisLabels={['Speed', 'Hull']} series={[{ slug: 'a', name: 'A', color: '#E8A23C', values: [1, 0.3] }]} />,
    );
    // 2 axes < 3 → null
    expect(container.querySelector('svg')).toBeNull();
  });
});
