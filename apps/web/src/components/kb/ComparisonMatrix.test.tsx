import React from 'react';
import { describe, it, expect, vi } from 'vitest';
import { render, screen, fireEvent } from '@testing-library/react';
import { ComparisonMatrix } from './ComparisonMatrix';
import type { ComparisonMatrix as MatrixModel } from '@/lib/kb-compare-types';

const model: MatrixModel = {
  columns: [
    { slug: 'avenger', display_name: 'Avenger', class_name: 'A', peer_group: 'combat', metrics: {} },
    { slug: 'gladius', display_name: 'Gladius', class_name: 'G', peer_group: 'combat', metrics: {} },
  ],
  rows: [
    { key: 'speed.scm', label: 'SCM speed', unit: 'm/s', group: 'Flight & handling', cells: [
      { value: 262, text: '262 m/s', fillPct: 100, isLeader: true },
      { value: 226, text: '226 m/s', fillPct: 0, isLeader: false },
    ] },
  ],
};

describe('ComparisonMatrix', () => {
  it('renders header columns (anchor first) + a sortable metric row, and the leader cell', () => {
    const onSort = vi.fn();
    render(<ComparisonMatrix model={model} sort={{ key: 'speed.scm', dir: 'desc' }} onSort={onSort} />);
    expect(screen.getByText('Avenger')).toBeTruthy();
    expect(screen.getByText('262 m/s')).toBeTruthy();
    fireEvent.click(screen.getByRole('button', { name: /sort by scm speed/i }));
    expect(onSort).toHaveBeenCalledWith('speed.scm');
  });
});
