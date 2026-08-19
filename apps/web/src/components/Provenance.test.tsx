import React from 'react';
import { describe, it, expect } from 'vitest';
import { render, screen } from '@testing-library/react';

import { Provenance } from './Provenance';

describe('Provenance', () => {
  it('renders nothing extra when the total is fully observed', () => {
    // The important half. Badging every number teaches people to ignore
    // the badge; the signal is only worth having while it is rare.
    const { container } = render(
      <Provenance total={12} inferred={0} note="x">
        12 deaths
      </Provenance>,
    );
    expect(screen.getByText('12 deaths')).toBeInTheDocument();
    expect(container.querySelector('[role="note"]')).toBeNull();
    expect(container.textContent).toBe('12 deaths');
  });

  it('marks the value and states the split when part of it is inferred', () => {
    render(
      <Provenance total={12} inferred={5} note="reconstructed from Corpse lines">
        12 deaths
      </Provenance>,
    );
    const marked = screen.getByRole('note');
    expect(marked).toHaveAttribute(
      'aria-label',
      '5 of 12 inferred, not observed — reconstructed from Corpse lines',
    );
    expect(marked).toHaveAttribute('title', expect.stringContaining('Corpse lines'));
  });

  it('explains WHY, not merely THAT — the note reaches the label', () => {
    // A provenance marker that cannot say how the inference was made is
    // an unexplained asterisk.
    render(
      <Provenance total={3} inferred={1} note="derived from session boundaries">
        3
      </Provenance>,
    );
    expect(screen.getByRole('note').getAttribute('aria-label')).toContain(
      'derived from session boundaries',
    );
  });

  it('does not broadcast an impossible split as fact', () => {
    // inferred > total means a bug upstream. Rendering "15 of 12" would
    // present that bug to the user as a finding.
    render(
      <Provenance total={12} inferred={15} note="x">
        12
      </Provenance>,
    );
    expect(screen.getByRole('note').getAttribute('aria-label')).not.toContain('15 of 12');
  });

  it('treats a non-finite split as no split', () => {
    const { container } = render(
      <Provenance total={12} inferred={NaN} note="x">
        12
      </Provenance>,
    );
    expect(container.querySelector('[role="note"]')).toBeNull();
  });
});
