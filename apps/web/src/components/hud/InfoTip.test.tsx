import React from 'react';
import { describe, it, expect } from 'vitest';
import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { InfoTip } from './InfoTip';

describe('InfoTip', () => {
  it('exposes the explanation via an accessible describedby association', () => {
    render(<InfoTip label="quantum jumps" text="Counted from target-selection events." />);
    const btn = screen.getByRole('button', { name: /how quantum jumps is calculated/i });
    const describedby = btn.getAttribute('aria-describedby');
    expect(describedby).toBeTruthy();
    const tip = document.getElementById(describedby!);
    expect(tip).not.toBeNull();
    expect(tip).toHaveTextContent('Counted from target-selection events.');
    expect(tip).toHaveAttribute('role', 'tooltip');
  });

  it('starts collapsed and opens on click, then closes on Escape', async () => {
    const user = userEvent.setup();
    render(<InfoTip text="Explanation." />);
    const btn = screen.getByRole('button');
    expect(btn).toHaveAttribute('aria-expanded', 'false');

    await user.click(btn);
    expect(btn).toHaveAttribute('aria-expanded', 'true');

    await user.keyboard('{Escape}');
    expect(btn).toHaveAttribute('aria-expanded', 'false');
  });

  it('opens on keyboard focus', async () => {
    const user = userEvent.setup();
    render(<InfoTip text="Explanation." />);
    const btn = screen.getByRole('button');
    await user.tab();
    expect(btn).toHaveFocus();
    expect(btn).toHaveAttribute('aria-expanded', 'true');
  });

  it('falls back to a generic label when no metric name is given', () => {
    render(<InfoTip text="x" />);
    expect(
      screen.getByRole('button', { name: /how this is calculated/i }),
    ).toBeInTheDocument();
  });
});
