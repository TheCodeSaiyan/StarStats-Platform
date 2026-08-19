import React from 'react';
import Link from 'next/link';
import type { ResolvedItem } from '@/lib/api';
import { prettify } from '@/lib/loadout';
import { ItemImage } from './ItemImage';

interface ItemTileProps {
  cls: string;
  port: string;
  resolved?: ResolvedItem;
}

export function ItemTile({ cls, resolved }: ItemTileProps) {
  const name = resolved?.display_name ?? prettify(cls);

  const nameNode =
    resolved?.slug != null ? (
      <Link href={`/kb/${resolved.category}/${resolved.slug}`}>{name}</Link>
    ) : (
      <span>{name}</span>
    );

  return (
    <div className="loadout-item-tile">
      {resolved?.has_image === true && (
        <ItemImage src={`/kb/media/${resolved.category}/${cls}/0`} alt={name} />
      )}
      <div className="loadout-item-tile__name">{nameNode}</div>
    </div>
  );
}
