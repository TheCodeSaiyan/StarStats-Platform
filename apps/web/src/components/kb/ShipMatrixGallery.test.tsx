import React from 'react';
import { afterEach, describe, expect, it } from 'vitest';
import { cleanup, fireEvent, render, screen } from '@testing-library/react';

import { ShipMatrixGallery } from './ShipMatrixGallery';

const URLS = ['/kb/media/vehicle/AEGS_X/0', '/kb/media/vehicle/AEGS_X/1'];

describe('ShipMatrixGallery', () => {
  afterEach(() => cleanup());

  it('renders nothing when there are no images', () => {
    const { container } = render(<ShipMatrixGallery mediaUrls={[]} />);
    expect(container.firstChild).toBeNull();
  });

  it('opens a lightbox on thumbnail click and pages through images', () => {
    render(<ShipMatrixGallery mediaUrls={URLS} />);
    // One expand button per image, no dialog yet.
    expect(screen.queryByRole('dialog')).toBeNull();
    fireEvent.click(screen.getByLabelText('Expand Ship Matrix image 1'));

    const dialog = screen.getByRole('dialog');
    expect(dialog).toBeInTheDocument();
    expect(screen.getByText('1 / 2')).toBeInTheDocument();

    fireEvent.click(screen.getByLabelText('Next image'));
    expect(screen.getByText('2 / 2')).toBeInTheDocument();

    // Wraps around back to the first.
    fireEvent.click(screen.getByLabelText('Next image'));
    expect(screen.getByText('1 / 2')).toBeInTheDocument();

    fireEvent.click(screen.getByLabelText('Close'));
    expect(screen.queryByRole('dialog')).toBeNull();
  });
});
