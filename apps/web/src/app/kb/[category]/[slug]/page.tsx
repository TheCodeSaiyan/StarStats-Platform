/**
 * Entity detail page. Fetches the full `ReferenceEntry` (with the
 * metadata blob the listing endpoint strips) via the per-slug
 * detail endpoint, then renders, top to bottom:
 *
 *   - A hero header: category eyebrow, large display name, raw
 *     class_name (mono chip), wiki link, decorative reticle ring.
 *   - "At a glance" — the curated summary fields (manufacturer /
 *     role / etc.) as a prominent stat strip.
 *   - Ship Matrix section (vehicles only) — specs + description +
 *     image gallery, when the enrichment blob is present. Rendered
 *     above the view so its RSI images show in both view modes.
 *   - `<KbDetailView>` — the client view that switches between the
 *     Visual presentation (peer-relative bars + handling radar +
 *     percentile callouts, sourced from the per-category `/stats`
 *     buckets) and the Compact grouped table, persisting the choice.
 *
 * M10 legal posture: this page renders FACTS + CIG data only. The
 * former wiki DESCRIPTION prose, the "View on Star Citizen Wiki" deep
 * link, and the verbatim "All raw fields" metadata dump were removed —
 * the only long-form copy left is the CIG-owned Ship Matrix description
 * (vehicles), shown under the CIG disclaimer. So nothing on this page
 * redistributes wiki-copyrightable prose.
 *
 * The per-slug detail fetch (`getEntityDetail`) is retained — it is NOT
 * the catalog listing that the M10 static cutover replaced; it supplies
 * the numeric spec metadata (facts) the bars/radar/comparison render.
 *
 * 404s for unknown category, missing slug, or any backend failure.
 */

import type { Metadata } from 'next';
import Link from 'next/link';
import type { Route } from 'next';
import { InstrumentStrip } from '@/components/hud/InstrumentStrip';
import { notFound } from 'next/navigation';
import { listContractsByEntity } from '@/lib/contracts';
import { RelatedContracts } from './_components/RelatedContracts';
import {
  getEntityDetail,
  getCategoryBundle,
  type ReferenceCategory,
  type Summary,
  tierLabel,
  subtypeLabel,
  placementLabel,
} from '@/lib/reference';
import {
  shipMatrixForCategory,
  shipMatrixMediaUrls,
} from '@/lib/ship-matrix';
import { ShipMatrixSection } from '@/components/kb/ShipMatrixSection';
import { getCategoryStats } from '@/lib/kb-stats';
import { KbDetailView } from '@/components/kb/KbDetailView';
import { getSession } from '@/lib/session';
import { getPreferences } from '@/lib/api';

const CATEGORY_LABELS: Record<ReferenceCategory, string> = {
  vehicle: 'Vehicles',
  weapon: 'Weapons',
  item: 'Items',
  location: 'Locations',
};

const VALID: ReadonlyArray<ReferenceCategory> = [
  'vehicle',
  'weapon',
  'item',
  'location',
];

function isCategory(s: string): s is ReferenceCategory {
  return (VALID as readonly string[]).includes(s);
}

interface PageProps {
  params: Promise<{ category: string; slug: string }>;
}

export async function generateMetadata(props: PageProps): Promise<Metadata> {
  const { category, slug } = await props.params;
  if (!isCategory(category)) return { title: 'Knowledge base' };
  const outcome = await getEntityDetail(category, slug);
  if (outcome.kind !== 'ok') return { title: 'Not found — Knowledge base' };
  return {
    title: `${outcome.entry.display_name} — ${CATEGORY_LABELS[category]}`,
  };
}

export default async function KbDetailPage(props: PageProps) {
  const { category, slug } = await props.params;
  if (!isCategory(category)) notFound();
  const outcome = await getEntityDetail(category, slug);
  // Only `not_found` collapses to the 404 page. A transient
  // backend error (`error`) throws so the Next error boundary can
  // render a "something went wrong, retry" surface instead of a
  // permanent 404 — the old `null`-collapsed shape was misleading
  // users into thinking the entity didn't exist when really the
  // backend hiccupped.
  if (outcome.kind === 'not_found') notFound();
  if (outcome.kind === 'error') {
    throw new Error(
      `Failed to load ${category}/${slug}: ${outcome.reason}`,
    );
  }
  const entry = outcome.entry;

  const summaryFields = summaryAtGlanceFields(entry.summary);

  // Vehicle-only Ship Matrix enrichment. `metadata` is opaque
  // (`Record<string, unknown>`) so the blob is validated at the
  // boundary; a missing / malformed `ship_matrix` collapses to `null`
  // and the section is skipped entirely. Media URLs are built
  // server-side (the API base is server-only) — the gallery degrades
  // to nothing when the blob carries no media, and any image the
  // backend kill-switch serves dark just fails to load.
  const shipMatrix = shipMatrixForCategory(category, entry.metadata);
  const shipMatrixMedia = shipMatrix
    ? shipMatrixMediaUrls(shipMatrix, entry.class_name)
    : [];

  // Peer-stats for the contextual view (degrades to empty on failure).
  const stats = await getCategoryStats(category);

  // Catalog for the comparison tray: all slugged entries in this category.
  const bundle = await getCategoryBundle(category);
  const catalog = bundle.list
    .filter((e) => e.slug)
    .map((e) => ({ slug: e.slug as string, display_name: e.display_name }));

  // Signed-in users get their persisted view config server-side to avoid a
  // flash; anonymous users reconcile from localStorage client-side.
  const session = await getSession();
  let serverPrefs: { view?: 'visual' | 'compact'; units?: 'metric' | 'imperial' } | null = null;
  if (session?.token) {
    try {
      const p = await getPreferences(session.token);
      serverPrefs = {
        view: p.kb_view as 'visual' | 'compact' | undefined,
        units: p.kb_units as 'metric' | 'imperial' | undefined,
      };
    } catch {
      serverPrefs = null;
    }
  }

  // M10: no free-text description prose on the detail page. The only
  // long-form copy we render is the CIG-owned Ship Matrix description
  // (vehicles only, under the CIG disclaimer). The wiki's `description`
  // metadata field — copyrightable prose — is NOT surfaced, so nothing
  // uncredited-CC-BY-SA ships. The visual view gets no `description`.
  const description = undefined;
  const roleTags: string[] = []; // Phase 1: role chips already in "At a glance".

  // Contracts referencing this entity. Best-effort: a contracts hiccup
  // must not break a KB page, matching how /kb guards its contract count.
  const relatedContracts = await listContractsByEntity(category, slug);

  return (
    <main
      style={{
        maxWidth: 920,
        display: 'flex',
        flexDirection: 'column',
        gap: 'var(--s5, 24px)',
      }}
    >
      <Link
        href={`/kb/${category}` as Route}
        // No prefetch: the target list page fetches the full category
        // bundle (≈4 MB), and every detail view would otherwise prefetch
        // it through the rate-limited reference API.
        prefetch={false}
        style={{
          fontSize: 13,
          color: 'var(--fg-muted)',
          textDecoration: 'none',
          width: 'fit-content',
        }}
      >
        ← Knowledge base · {CATEGORY_LABELS[category]}
      </Link>

      {/* Hero. Decorative reticle ring (original primitive, brand-safe)
          sits behind the title as atmosphere — not an RSI asset. */}
      <div
        className="ss-card"
        style={{
          position: 'relative',
          overflow: 'hidden',
          padding: '24px 24px 22px',
          background:
            'linear-gradient(180deg, var(--surface-2, #221F2A), var(--bg-elev, #15131A))',
          borderColor: 'var(--border-strong, rgba(255,255,255,0.14))',
        }}
      >
        <div
          aria-hidden
          style={{
            position: 'absolute',
            top: -70,
            right: -70,
            width: 220,
            height: 220,
            borderRadius: '50%',
            border: '1px solid var(--accent-soft, rgba(232,162,60,0.14))',
            boxShadow: 'inset 0 0 0 24px var(--accent-soft, rgba(232,162,60,0.06))',
            opacity: 0.6,
            pointerEvents: 'none',
          }}
        />
        <InstrumentStrip
          size="hero"
          title={
            <h1 className="hud-tile__title" style={{ margin: 0, fontSize: 'inherit' }}>
              {entry.display_name}
            </h1>
          }
          context={CATEGORY_LABELS[category]}
        />
        <div
          style={{
            display: 'flex',
            flexWrap: 'wrap',
            alignItems: 'center',
            gap: 10,
            marginTop: 12,
          }}
        >
          <code
            className="mono"
            style={{
              fontSize: 12,
              color: 'var(--fg-muted)',
              background: 'var(--surface, #1A1820)',
              border: '1px solid var(--border, rgba(255,255,255,0.07))',
              borderRadius: 'var(--r-sm, 6px)',
              padding: '3px 8px',
              wordBreak: 'break-all',
            }}
          >
            {entry.class_name}
          </code>
        </div>
      </div>

      {summaryFields.length > 0 && (
        <section
          className="ss-card"
          style={{ padding: '18px 20px' }}
        >
          <h2
            style={{
              margin: '0 0 14px',
              fontSize: 14,
              fontWeight: 600,
              color: 'var(--fg)',
            }}
          >
            At a glance
          </h2>
          <dl
            style={{
              display: 'grid',
              gridTemplateColumns: 'repeat(auto-fit, minmax(160px, 1fr))',
              gap: 18,
              margin: 0,
            }}
          >
            {summaryFields.map(([k, v]) => (
              <div key={k}>
                <dt
                  className="mono"
                  style={{
                    color: 'var(--fg-muted)',
                    fontSize: 11,
                    textTransform: 'uppercase',
                    letterSpacing: '0.08em',
                  }}
                >
                  {humanLabel(k)}
                </dt>
                <dd
                  style={{
                    margin: '5px 0 0',
                    fontSize: 16,
                    color: 'var(--fg)',
                    lineHeight: 1.3,
                  }}
                >
                  {v}
                </dd>
              </div>
            ))}
          </dl>
        </section>
      )}

      {shipMatrix && (
        <ShipMatrixSection
          shipMatrix={shipMatrix}
          mediaUrls={shipMatrixMedia}
        />
      )}

      <KbDetailView
        category={category}
        displayName={entry.display_name}
        metadata={entry.metadata as Record<string, unknown>}
        groups={stats.groups}
        cohorts={entry.cohorts ?? []}
        description={description}
        roleTags={roleTags}
        serverPrefs={serverPrefs}
        signedIn={!!session?.token}
        anchorSlug={entry.slug ?? ''}
        catalog={catalog}
      />

      <RelatedContracts contracts={relatedContracts} />
    </main>
  );
}

/** Convert `manufacturer` → `Manufacturer`, `hull_size` → `Hull size`. */
function humanLabel(key: string): string {
  return key
    .replace(/_/g, ' ')
    .replace(/^./, (c) => c.toUpperCase());
}

/** Per-category at-a-glance pairs. Excludes the `category`
 *  discriminator since it's redundant with the page's URL. */
function summaryAtGlanceFields(summary: Summary): Array<[string, string]> {
  const out: Array<[string, string]> = [];
  const push = (label: string, value: string | undefined) => {
    if (value && value.length > 0) out.push([label, value]);
  };
  switch (summary.category) {
    case 'vehicle':
      push('manufacturer', summary.manufacturer);
      push('role', summary.role);
      push('hull_size', summary.hull_size);
      push('focus', summary.focus);
      break;
    case 'weapon':
      push('manufacturer', summary.manufacturer);
      push('weapon_type', summary.weapon_type);
      push('size', summary.size);
      push('damage_type', summary.damage_type);
      break;
    case 'item':
      push('manufacturer', summary.manufacturer);
      push('item_type', summary.item_type);
      push('grade', summary.grade);
      break;
    case 'location':
      // Wave 2 taxonomy fields first, mirroring EntityHoverCard's
      // ordering — so the detail page and hover-card stay in sync.
      if (summary.tier) push('tier', tierLabel(summary.tier));
      if (summary.subtype) push('subtype', subtypeLabel(summary.subtype));
      if (summary.placement) push('placement', placementLabel(summary.placement));
      push('system', summary.system);
      push('parent', summary.parent);
      // Suppress legacy classification when the richer subtype is
      // populated — same de-duplication rule as the hover card.
      if (!summary.subtype) push('classification', summary.classification);
      push('operator', summary.operator);
      push('faction', summary.faction);
      push('tag', summary.tag);
      break;
  }
  return out;
}
