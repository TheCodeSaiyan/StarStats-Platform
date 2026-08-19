import React from 'react';
import { describe, it, expect } from 'vitest';
import { render, fireEvent } from '@testing-library/react';
import { ItemImage } from './ItemImage';

describe('ItemImage', () => {
  it('renders the image with the proxied src + alt', () => {
    const { getByAltText } = render(
      <ItemImage src="/kb/media/item/helmet_x/0" alt="The Butcher Helmet" />,
    );
    const img = getByAltText('The Butcher Helmet');
    expect(img).toHaveAttribute('src', '/kb/media/item/helmet_x/0');
    expect(img).toHaveClass('loadout-item-tile__img');
  });

  it('drops the image (renders nothing) when it fails to load', () => {
    const { queryByAltText, container } = render(
      <ItemImage src="/kb/media/item/missing/0" alt="Missing Item" />,
    );
    const img = queryByAltText('Missing Item');
    expect(img).not.toBeNull();
    fireEvent.error(img!);
    // After an error the image is removed — no broken-image icon, tile
    // falls back to name-only (matching the has_image:false case).
    expect(queryByAltText('Missing Item')).toBeNull();
    expect(container.querySelector('img')).toBeNull();
  });
});
