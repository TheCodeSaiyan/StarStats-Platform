/**
 * Tray-side equivalent of the web app's `<EntityLink>`. Renders an
 * entity identifier as either:
 *   - a plain `<span>` (no catalog match, no slug, or no webOrigin
 *     to build the KB URL); or
 *   - a clickable button that opens `${webOrigin}/kb/{category}/{slug}`
 *     in the user's default browser via `@tauri-apps/plugin-shell`'s
 *     `open()`.
 *
 * The tray is a Tauri WebView, not a browser tab — Next.js `<Link>`
 * doesn't apply. Anchors with `target="_blank"` would render but the
 * WebView's CSP blocks the navigation, so we use the shell plugin
 * exactly like the existing surfaces in `KbPane` and `StatusPane`.
 *
 * Hover popover is intentionally skipped for v1 — the surface area
 * the tray covers (mostly Status / Logs row text) is too narrow for
 * a popover to be ergonomic. The web app keeps the EntityHoverCard
 * pattern for its broader rendering surfaces.
 */

import { open as openShell } from '@tauri-apps/plugin-shell';
import type {
  ReferenceCatalog,
  ReferenceCategory,
  ReferenceEntry,
} from '../../lib/reference';
import { resolveReferenceEntry, webKbUrl } from '../../lib/reference';
import { TierChip } from './TierChip';

interface Props {
  /** Which catalog category the identifier belongs to. */
  category: ReferenceCategory;
  /** The raw class identifier from the event payload (e.g.
   *  `AEGS_Avenger_Stalker`). Case-insensitive lookup against
   *  `catalog`. */
  classKey: string | null | undefined;
  /** Catalog for this category. Pass the matching
   *  `bundles[category].catalog` from `loadAllReferenceBundles`.
   *  Optional — without it the component always renders the plain
   *  fallback. */
  catalog?: ReferenceCatalog;
  /** Override the displayed string. Defaults to the catalog's
   *  `display_name` (or the raw `classKey` if no match). */
  label?: string;
  /** Paired API's companion web origin from `config.web_origin`.
   *  Required to produce the navigable URL; without it the
   *  component falls back to plain text. */
  webOrigin?: string | null;
  /** When `category === 'location'` and the resolved entry carries
   *  a taxonomy v2 tier, render a `<TierChip>` after the link/text.
   *  Opt-in — dense surfaces leave it off; hero/pill/topbar
   *  surfaces enable it. Mirrors the web `<EntityLink>` API. */
  showTier?: boolean;
  /** Tier-chip variant when `showTier` is enabled. `compact` hides
   *  the tier label and renders only the subtype. */
  tierChipVariant?: 'full' | 'compact';
  /** Pre-resolved entity from the client-side classifier (Rust). When
   *  provided, the catalog lookup is skipped and this drives the link
   *  directly: `slug` present → link to `/kb/{category}/{slug}`, else
   *  plain `displayName`. Used by timeline rows where Rust already
   *  resolved the location. */
  resolved?: { slug?: string | null; displayName: string };
}

export function TrayEntityLink({
  category,
  classKey,
  catalog,
  label,
  webOrigin,
  showTier = false,
  tierChipVariant = 'full',
  resolved,
}: Props) {
  // Two resolution modes: a pre-resolved client-side result (skip the
  // catalog), or a catalog lookup against `classKey`.
  if (!classKey && !resolved) {
    return <span>{label ?? ''}</span>;
  }

  const entry: ReferenceEntry | undefined = resolved
    ? undefined
    : resolveReferenceEntry(category, classKey, catalog);

  const slug = resolved ? resolved.slug ?? null : entry?.slug ?? null;
  const text =
    label ?? resolved?.displayName ?? entry?.display_name ?? classKey ?? '';

  // Tier chip is opt-in and only meaningful for catalog-resolved
  // location entries with Wave 2 taxonomy populated.
  const tierNode =
    !resolved &&
    showTier &&
    entry?.summary.category === 'location' &&
    entry.summary.tier ? (
      <>
        {' '}
        <TierChip
          tier={entry.summary.tier}
          subtype={entry.summary.subtype}
          compact={tierChipVariant === 'compact'}
        />
      </>
    ) : null;

  // Without a slug or webOrigin we can't build a URL — render plain
  // text rather than a dead button.
  if (!slug || !webOrigin) {
    return (
      <span>
        {text}
        {tierNode}
      </span>
    );
  }

  const href = resolved
    ? `${webOrigin.replace(/\/+$/, '')}/kb/${category}/${slug}`
    : webKbUrl(webOrigin, category, entry!);

  return (
    <span style={{ display: 'inline-flex', alignItems: 'baseline', gap: 6 }}>
      <button
        type="button"
        onClick={() => {
          void openShell(href);
        }}
        title={`Open ${text} in StarStats web`}
        style={{
          background: 'transparent',
          border: 'none',
          padding: 0,
          margin: 0,
          font: 'inherit',
          color: 'var(--accent)',
          borderBottom: '1px dotted var(--accent)',
          cursor: 'pointer',
        }}
      >
        {text}
      </button>
      {tierNode}
    </span>
  );
}
