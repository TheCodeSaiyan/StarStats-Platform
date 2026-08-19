import React from 'react';
import { afterEach, describe, expect, it } from 'vitest';
import { cleanup, render, screen } from '@testing-library/react';

import { ShipMatrixDisclaimer } from './ShipMatrixDisclaimer';

afterEach(() => {
  cleanup();
});

describe('ShipMatrixDisclaimer', () => {
  it('renders the verbatim CIG attribution wording', () => {
    render(<ShipMatrixDisclaimer />);
    expect(
      screen.getByText(
        /Ship specifications, descriptions and images © Cloud Imperium Rights LLC \/ Cloud Imperium Rights Ltd\. StarStats is an unofficial fan site, not endorsed by or affiliated with Cloud Imperium Group\./,
      ),
    ).toBeInTheDocument();
  });

  it('is exposed as a labelled landmark region for assistive tech', () => {
    const { container } = render(<ShipMatrixDisclaimer />);
    const aside = container.querySelector(
      'aside[aria-label="Ship Matrix attribution"]',
    );
    expect(aside).not.toBeNull();
  });
});
