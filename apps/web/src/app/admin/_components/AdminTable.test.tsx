// Explicit React import: this repo's vitest uses the classic JSX
// runtime, so a JSX-rendering component ReferenceErrors without it.
import React from 'react';
import { describe, expect, it } from 'vitest';
import { render, screen } from '@testing-library/react';
import { AdminTable } from './AdminTable';

interface Row {
  id: string;
  name: string;
}

const COLUMNS = [
  { header: 'Name', cell: (r: Row) => r.name },
  { header: 'Id', cell: (r: Row) => r.id },
];

describe('AdminTable', () => {
  it('renders a header cell per column', () => {
    render(
      <AdminTable
        columns={COLUMNS}
        rows={[{ id: 'a', name: 'Alpha' }]}
        rowKey={(r) => r.id}
        empty="None."
      />,
    );
    expect(screen.getByText('Name')).toBeInTheDocument();
    expect(screen.getByText('Id')).toBeInTheDocument();
  });

  it('renders the empty message and no table when rows is empty', () => {
    render(
      <AdminTable
        columns={COLUMNS}
        rows={[]}
        rowKey={(r) => r.id}
        empty="No users match this search."
      />,
    );
    expect(screen.getByText('No users match this search.')).toBeInTheDocument();
    expect(screen.queryByRole('table')).toBeNull();
  });

  it('renders one row per item with each column cell', () => {
    render(
      <AdminTable
        columns={COLUMNS}
        rows={[
          { id: 'a', name: 'Alpha' },
          { id: 'b', name: 'Beta' },
        ]}
        rowKey={(r) => r.id}
        empty="None."
      />,
    );
    // header row + 2 data rows
    expect(screen.getAllByRole('row')).toHaveLength(3);
    expect(screen.getByText('Alpha')).toBeInTheDocument();
    expect(screen.getByText('Beta')).toBeInTheDocument();
  });

  it('renders ReactNode cells, not just strings', () => {
    render(
      <AdminTable
        columns={[
          { header: 'Name', cell: (r: Row) => <strong>{r.name}</strong> },
        ]}
        rows={[{ id: 'a', name: 'Alpha' }]}
        rowKey={(r) => r.id}
        empty="None."
      />,
    );
    expect(screen.getByText('Alpha').tagName).toBe('STRONG');
  });
});
