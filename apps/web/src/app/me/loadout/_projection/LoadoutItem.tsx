import React from 'react';
import Link from 'next/link';
import type { Route } from 'next';
import type { ResolvedItem } from '@/lib/api';
import { prettify } from '@/lib/loadout';
import { ItemImage } from '../_components/ItemImage';

/**
 * One piece of kit: thumbnail plus name.
 *
 * IMAGES ARE SAME-ORIGIN ONLY. The src is always the app's own
 * `/kb/media/{category}/{class}/{idx}` proxy, never a raw
 * `media.starcitizen.tools` URL — the proxy is what enforces the SSRF
 * allowlist. `ItemImage` is a client wrapper that drops the image on load
 * error, because an item's KB metadata can advertise `has_image: true` for a
 * picture that is genuinely gone upstream, and a bare `<img>` would then show
 * the browser's broken-image glyph. Missing degrades to name-only.
 *
 * The name links only where the item resolved to a KB slug; an unresolved
 * class falls back to `prettify` rather than showing the raw engine id.
 */
export function LoadoutItem({
  cls,
  resolved,
}: {
  cls: string;
  port: string;
  resolved?: ResolvedItem;
}) {
  const name = resolved?.display_name ?? prettify(cls);

  return (
    <div className="hp-kit">
      {resolved?.has_image === true ? (
        <ItemImage
          src={`/kb/media/${resolved.category}/${cls}/0`}
          alt={name}
          className="hp-kit__img"
        />
      ) : null}
      <div className="hp-kit__name">
        {resolved?.slug != null ? (
          <Link href={`/kb/${resolved.category}/${resolved.slug}` as Route}>
            {name}
          </Link>
        ) : (
          <span>{name}</span>
        )}
      </div>
    </div>
  );
}
