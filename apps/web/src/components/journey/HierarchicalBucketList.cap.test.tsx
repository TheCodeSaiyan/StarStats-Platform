import React from 'react';
import { describe, it, expect } from 'vitest';
import { render } from '@testing-library/react';
import { HierarchicalBucketList, type RollupNode } from './HierarchicalBucketList';

const nodes = (n: number): RollupNode[] =>
  Array.from({ length: n }, (_, i) => ({ label: `Node ${i}`, count: 100 - i }));

describe('HierarchicalBucketList — maxNodes cap', () => {
  it('shows every node and no "+N more" note when uncapped', () => {
    const { container } = render(<HierarchicalBucketList nodes={nodes(6)} />);
    expect(container.querySelectorAll('ol > li').length).toBe(6);
    expect(container.textContent).not.toContain('more');
  });

  it('caps to maxNodes and surfaces the remainder as a "+N more" note (never scrolls)', () => {
    const { container } = render(
      <HierarchicalBucketList nodes={nodes(6)} maxNodes={4} />,
    );
    // 4 shown rows + 1 "+N more" note row.
    expect(container.querySelectorAll('ol > li').length).toBe(5);
    expect(container.textContent).toContain('+2 more');
    expect(container.textContent).not.toContain('Node 5');
  });

  it('shows no note when maxNodes exceeds the node count', () => {
    const { container } = render(
      <HierarchicalBucketList nodes={nodes(3)} maxNodes={10} />,
    );
    expect(container.querySelectorAll('ol > li').length).toBe(3);
    expect(container.textContent).not.toContain('more');
  });
});
