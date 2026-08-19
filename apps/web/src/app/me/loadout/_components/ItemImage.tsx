'use client';

import React, { useState } from 'react';

interface ItemImageProps {
  src: string;
  alt: string;
}

/**
 * Item thumbnail with graceful failure handling.
 *
 * An item's KB metadata can advertise an image (`has_image`) that is
 * nonetheless genuinely missing upstream — a dead `media.starcitizen.tools`
 * link makes the proxy return 404. A plain server-rendered `<img>` would
 * then show the browser's broken-image icon. This client wrapper drops the
 * image on load error so the tile falls back to name-only, matching the
 * `has_image: false` case.
 */
export function ItemImage({ src, alt }: ItemImageProps) {
  const [failed, setFailed] = useState(false);
  if (failed) return null;
  return (
    <img
      src={src}
      alt={alt}
      className="loadout-item-tile__img"
      onError={() => setFailed(true)}
    />
  );
}
