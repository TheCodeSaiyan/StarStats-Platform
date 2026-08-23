/**
 * Entity detail sheet, in the projection.
 *
 * COVERAGE BOUNDARY. The kit read `/kb/[category]/page.tsx` for the BROWSE and
 * never read this route, so nothing here is grounded in a kit screen — it is
 * ported from the flat page itself. Order, copy and field selection are
 * unchanged; only the frame is.
 *
 * Top to bottom:
 *
 *   - Pane 1, titled with the entry's display name: the raw `class_name`
 *     verbatim, then "At a glance" — the curated summary fields — and, for a
 *     vehicle whose enrichment blob parsed, the Ship Matrix specs + CIG
 *     description + image gallery.
 *   - Pane 2, "Statistics": `<KbDetailView>`, the client view that switches
 *     between the Visual presentation (peer-relative bars + handling radar +
 *     percentile callouts, from the per-category `/stats` buckets) and the
 *     Compact grouped table, plus units and the cohort comparison tray.
 *   - Pane 3, "Contracts": rendered only when contracts reference this entity.
 *
 * `KbDetailView`, `ShipMatrixSection` and `RelatedContracts` are NOT rewritten.
 * They are written against the flat semantic tokens (`--fg`, `--border`,
 * `--accent`, `--ok`), and the projection's token layer ALIASES those onto the
 * beam, so their colours land correctly; the flat-primitive bridge in the
 * pattern layer squares off their `.ss-card` frames. What that leaves is the
 * comparison system — a large amount of tested behaviour whose reimplementation
 * would risk far more than a redrawn border gains. It is the one place in this
 * port where flat components render inside a projection surface, and it is a
 * deliberate staging point, not an end state.
 *
 * M10 legal posture: this page renders FACTS + CIG data only. The former wiki
 * DESCRIPTION prose, the "View on Star Citizen Wiki" deep link, and the
 * verbatim "All raw fields" metadata dump were removed — the only long-form
 * copy left is the CIG-owned Ship Matrix description (vehicles), shown under
 * the CIG disclaimer. So nothing on this page redistributes wiki-copyrightable
 * prose.
 *
 * The per-slug detail fetch (`getEntityDetail`) is retained — it is NOT the
 * catalog listing that the M10 static cutover replaced; it supplies the numeric
 * spec metadata (facts) the bars/radar/comparison render.
 *
 * 404s for unknown category, missing slug, or any backend failure.
 */

import type { Metadata } from 'next';
import { Plane, HoloKV, type Calibration } from 'holo';
import { getTheme } from '@/lib/theme';
import { navSections } from '@/lib/nav';
import { setCalibrationAction } from '@/app/me/_projection/actions';
import { KbProjection, type KbSection } from '../../_projection/KbProjection';
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

  let calibration: Calibration = 'terra';
  try {
    calibration = (await getTheme(session?.token)) as Calibration;
  } catch {
    // Preference read failed; the default stands.
  }

  const sections: KbSection[] = [
    {
      id: 'entry',
      title: entry.display_name,
      ctx: CATEGORY_LABELS[category],
      group: 'kb',
      node: (
        <>
          {/* The raw engine identifier, verbatim. It is what the log says and
              what a reader searches by, so it is never prettified. */}
          <div className="hp-secret hp-secret--uri">{entry.class_name}</div>

          {summaryFields.length > 0 ? (
            <Plane tilt="flat" cap="At a glance" style={{ marginTop: 20 }}>
              <HoloKV
                items={summaryFields.map(([k, v]) => ({
                  k: humanLabel(k),
                  v,
                }))}
              />
            </Plane>
          ) : null}
        </>
      ),
    },

    ...(shipMatrix
      ? [
          {
            id: 'ship-matrix',
            title: 'Ship Matrix',
            ctx: "Official specifications from RSI's Ship Matrix.",
            group: 'kb',
            node: (
              <ShipMatrixSection
                shipMatrix={shipMatrix}
                mediaUrls={shipMatrixMedia}
                heading={false}
              />
            ),
          } satisfies KbSection,
        ]
      : []),

    {
      id: 'stats',
      title: 'Statistics',
      ctx: 'Compared against peers',
      group: 'kb',
      // `KbDetailView` is a client component with its own view/units toggle,
      // comparison tray and cohort picker. Its inner charts are written against
      // the flat semantic tokens, which the projection ALIASES onto the beam —
      // so they land correctly coloured; the bridge in the pattern layer squares
      // off their card wrappers. Not rewritten: the comparison system is a lot
      // of tested behaviour, and re-implementing it to change its frame would
      // risk far more than it gains.
      node: (
        <KbDetailView
          category={category}
          metadata={entry.metadata as Record<string, unknown>}
          groups={stats.groups}
          cohorts={entry.cohorts ?? []}
          anchorSlug={entry.slug ?? ''}
          displayName={entry.display_name}
          catalog={catalog}
          serverPrefs={serverPrefs}
          signedIn={!!session?.token}
          description={description}
          roleTags={roleTags}
        />
      ),
    },

    // Gated on the SAME emptiness test the component applies internally.
    // `RelatedContracts` returns null for an unreferenced entry — most of them
    // are — and without this gate the projection would wrap that nothing in a
    // permanent empty pane, which is precisely the noise the component's own
    // comment exists to avoid.
    ...(relatedContracts.length > 0
      ? [
          {
            id: 'contracts',
            // The component's own wording, verbatim; it is suppressed inside
            // the pane so the words appear once, in the pane header.
            title: 'Contracts',
            group: 'kb',
            node: (
              <RelatedContracts contracts={relatedContracts} heading={false} />
            ),
          } satisfies KbSection,
        ]
      : []),
  ];

  return (
    <KbProjection
      handle={session?.claimedHandle}
      calibration={calibration}
      nav={navSections(
        { signedIn: Boolean(session), staffRoles: session?.staffRoles },
        'kb',
      )}
      crumb={[
        { label: 'Knowledge base', href: '/kb' },
        { label: CATEGORY_LABELS[category], href: `/kb/${category}` },
        { label: entry.display_name },
      ]}
      sections={sections}
      notice={null}
      onCalibrate={async (id: string) => {
        'use server';
        await setCalibrationAction(id);
      }}
    />
  );
}

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
