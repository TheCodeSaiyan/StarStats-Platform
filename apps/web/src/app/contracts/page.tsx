/**
 * Public contract browse. Free-text search + faceted filters (type,
 * faction, legal status, issuer) + sort, over the contracts ingested
 * from sp-ingest.
 *
 * Server component. Free-text `q` goes through the server `/search`
 * endpoint (rich blob: names, issuer, locations, objectives, steps).
 * Facet vocabularies + facet filtering + sort + pagination run in
 * memory over the full set (small dataset; one paged fetch), so the
 * chips always show the complete vocabulary and compose with search.
 */

import React from 'react';
import type { Metadata } from 'next';
import Link from 'next/link';
import type { Route } from 'next';
import {
  listContracts,
  listAllContracts,
  applyContractFacets,
  sortContractSummaries,
  distinctFacetValues,
  type ContractSummary,
  type ContractSortKey,
} from '@/lib/contracts';
import { InstrumentStrip } from '@/components/hud/InstrumentStrip';
import { ControlStrip } from '@/components/hud/ControlStrip';
import { AppSurface } from '@/components/projection/AppSurface';
import { RecordsIndex } from '@/components/projection/RecordsIndex';
import { getSession } from '@/lib/session';
import { getTheme } from '@/lib/theme';
import { navSections } from '@/lib/nav';
import { setCalibrationAction } from '@/app/me/_projection/actions';
import type { Calibration } from 'holo';

export const metadata: Metadata = {
  title: 'Contracts',
  description: 'Browse and search Star Citizen contracts.',
};

const PAGE_SIZE = 48;

const SORTS: Array<{ key: ContractSortKey; label: string }> = [
  { key: 'updated', label: 'Recent' },
  { key: 'name', label: 'Name' },
  { key: 'reward', label: 'Reward' },
];

interface PageProps {
  searchParams: Promise<{
    q?: string;
    type?: string;
    issuer?: string;
    faction?: string;
    legal_status?: string;
    gameplay_loop?: string;
    sort?: string;
    offset?: string;
  }>;
}

export default async function ContractsPage(props: PageProps) {
  // PUBLIC ROUTE — the catalogue reads the same for a stranger. The session is
  // read only so the chrome knows whether to offer an account menu or a Sign
  // in; nothing below it is gated on one.
  const session = await getSession();
  let calibration: Calibration = 'terra';
  try {
    calibration = (await getTheme(session?.token)) as Calibration;
  } catch {
    // Preference read failed; the default stands.
  }

  const sp = await props.searchParams;
  const q = (sp.q ?? '').trim();
  const type = (sp.type ?? '').trim();
  const issuer = (sp.issuer ?? '').trim();
  const faction = (sp.faction ?? '').trim();
  const legalStatus = (sp.legal_status ?? '').trim();
  const gameplayLoop = (sp.gameplay_loop ?? '').trim();
  const sort: ContractSortKey =
    sp.sort === 'name' || sp.sort === 'reward' ? sp.sort : 'updated';
  const offset = parsePositiveInt(sp.offset);

  // Full set drives the facet vocabularies (so chips always show every
  // possible value regardless of the current filter). When a search
  // term is present, the result base is the server's rich `/search`
  // (locations/objectives/steps); otherwise it's the full set.
  const all = await listAllContracts();
  const base = q ? (await listContracts({ q, limit: 200 })).contracts : all;

  const filtered = applyContractFacets(base, { type, issuer, faction, legalStatus, gameplayLoop });
  const sorted = sortContractSummaries(filtered, sort);
  const page = sorted.slice(offset, offset + PAGE_SIZE);

  const facets = {
    type: distinctFacetValues(all, 'contract_type'),
    faction: distinctFacetValues(all, 'faction'),
    legal_status: distinctFacetValues(all, 'legal_status'),
    gameplay_loop: distinctFacetValues(all, 'gameplay_loop'),
    issuer: distinctFacetValues(all, 'issuer'),
  };

  const current: Record<string, string> = {};
  if (q) current.q = q;
  if (type) current.type = type;
  if (issuer) current.issuer = issuer;
  if (faction) current.faction = faction;
  if (legalStatus) current.legal_status = legalStatus;
  if (gameplayLoop) current.gameplay_loop = gameplayLoop;
  if (sort !== 'updated') current.sort = sort;

  const buildHref = (overrides: Record<string, string | undefined>): Route => {
    const qs = new URLSearchParams(current);
    for (const [k, v] of Object.entries(overrides)) {
      if (v === undefined || v === '') qs.delete(k);
      else qs.set(k, v);
    }
    // Any change other than paging resets to the first page.
    if (!('offset' in overrides)) qs.delete('offset');
    const s = qs.toString();
    return (s ? `/contracts?${s}` : '/contracts') as Route;
  };

  const anyFilter = Boolean(q || type || issuer || faction || legalStatus || gameplayLoop);
  const context = `${filtered.length.toLocaleString()} of ${all.length.toLocaleString()}${q ? ` matching "${q}"` : ''}`;

  return (
    <AppSurface
      // Public surface — it carries the CIG trademark plate. `AppSurface`
      // serves both public and signed-in pages, so the caller decides.
      legal
      handle={session?.claimedHandle}
      calibration={calibration}
      nav={navSections({
        signedIn: Boolean(session),
        staffRoles: session?.staffRoles,
      })}
      crumb={[
        { label: 'Site', href: '/' },
        { label: 'Contract catalogue' },
      ]}
      sections={[
        {
          id: 'page',
          group: 'page',
          title: 'Contract catalogue',
          ctx: 'Every contract the parser has published',
          node: (
            <>
              {/* `Records.jsx` puts a pilot's own records behind one category
                  strip so they read as a family; the product had four unrelated
                  routes, each a dead end. */}
              <RecordsIndex active="/contracts" />
    <div>
      <InstrumentStrip
        title={
          <h1 className="hud-tile__title" style={{ margin: 0, fontSize: 18 }}>
            Contracts
          </h1>
        }
        context={context}
      />

      <ControlStrip>
        {/* Free-text search — routes to the server /search (rich blob). */}
        <form
          method="GET"
          action="/contracts"
          style={{ display: 'flex', gap: 8, marginTop: 16, flexWrap: 'wrap' }}
        >
          <input
            type="search"
            name="q"
            defaultValue={q}
            placeholder="Search name, issuer, location, objectives…"
            autoComplete="off"
            // Lit underline, never a boxed field — the primitive redraw
            // handles the rest via `.hp-stage input`.
            className="hp-input"
            style={{ flex: '1 1 280px' }}
          />
          {/* Preserve facets + sort across a search submit (the form only
              posts its own field, so they'd otherwise reset). */}
          {type && <input type="hidden" name="type" value={type} />}
          {issuer && <input type="hidden" name="issuer" value={issuer} />}
          {faction && <input type="hidden" name="faction" value={faction} />}
          {legalStatus && <input type="hidden" name="legal_status" value={legalStatus} />}
          {gameplayLoop && <input type="hidden" name="gameplay_loop" value={gameplayLoop} />}
          {sort !== 'updated' && <input type="hidden" name="sort" value={sort} />}
          <button type="submit" className="ss-btn ss-btn--primary">
            Search
          </button>
          {anyFilter && (
            <Link
              href={'/contracts' as Route}
              className="ss-btn ss-btn--ghost"
              prefetch={false}
              style={{ textDecoration: 'none' }}
            >
              Clear all
            </Link>
          )}
        </form>

        {/* Sort */}
        <div style={rowStyle} aria-label="Sort contracts">
          <span style={rowLabelStyle}>Sort</span>
          {SORTS.map((s) => (
            <ChipLink
              key={s.key}
              href={buildHref({ sort: s.key === 'updated' ? undefined : s.key })}
              active={sort === s.key}
              label={s.label}
            />
          ))}
        </div>

        {/* Facets */}
        <FacetRow label="Type" paramKey="type" values={facets.type} active={type} buildHref={buildHref} />
        <FacetRow label="Faction" paramKey="faction" values={facets.faction} active={faction} buildHref={buildHref} />
        <FacetRow
          label="Legal"
          paramKey="legal_status"
          values={facets.legal_status}
          active={legalStatus}
          buildHref={buildHref}
        />
        <FacetRow
          label="Loop"
          paramKey="gameplay_loop"
          values={facets.gameplay_loop}
          active={gameplayLoop}
          buildHref={buildHref}
        />
        <FacetRow label="Issuer" paramKey="issuer" values={facets.issuer} active={issuer} buildHref={buildHref} />
      </ControlStrip>

      <section
        style={{
          display: 'grid',
          gridTemplateColumns: 'repeat(auto-fit, minmax(240px, 1fr))',
          gap: 12,
          marginTop: 16,
        }}
      >
        {page.length === 0 ? (
          <p style={{ color: 'var(--fg-dim)', fontSize: 13 }}>
            No contracts match this filter.
          </p>
        ) : (
          page.map((c) => <ContractCard key={c.canonical_id} contract={c} />)
        )}
      </section>

      {sorted.length > PAGE_SIZE && (
        <nav
          style={{
            display: 'flex',
            justifyContent: 'space-between',
            gap: 12,
            marginTop: 20,
            flexWrap: 'wrap',
          }}
        >
          <span style={{ color: 'var(--fg-muted)', fontSize: 13 }}>
            {offset + 1}–{offset + page.length} of {sorted.length}
          </span>
          <div style={{ display: 'flex', gap: 8 }}>
            <PagerLink
              label="← Prev"
              href={offset > 0 ? buildHref({ offset: String(Math.max(0, offset - PAGE_SIZE)) }) : null}
            />
            <PagerLink
              label="Next →"
              href={offset + PAGE_SIZE < sorted.length ? buildHref({ offset: String(offset + PAGE_SIZE) }) : null}
            />
          </div>
        </nav>
      )}
    </div>
            </>
          ),
        },
      ]}
      notice={null}
      onCalibrate={async (id: string) => {
        'use server';
        await setCalibrationAction(id);
      }}
    />
  );
}

// ---------------------------------------------------------------------

function FacetRow({
  label,
  paramKey,
  values,
  active,
  buildHref,
}: {
  label: string;
  paramKey: string;
  values: string[];
  active: string;
  buildHref: (o: Record<string, string | undefined>) => Route;
}) {
  if (values.length < 2) return null; // nothing to choose between
  return (
    <div style={rowStyle} aria-label={`Filter by ${label.toLowerCase()}`}>
      <span style={rowLabelStyle}>{label}</span>
      {values.map((v) => {
        const isActive = active.toLowerCase() === v.toLowerCase();
        return (
          <ChipLink
            key={v}
            href={buildHref({ [paramKey]: isActive ? undefined : v })}
            active={isActive}
            label={v}
          />
        );
      })}
    </div>
  );
}

function ChipLink({ href, active, label }: { href: Route; active: boolean; label: string }) {
  return (
    <Link
      href={href}
      prefetch={false}
      data-active={active ? 'true' : undefined}
      // The catalogue's chip — one chip style across the product, and never
      // a FILLED one for the active state.
      className="hp-catchip"
    >
      {label}
    </Link>
  );
}

function PagerLink({ label, href }: { label: string; href: Route | null }) {
  if (!href) {
    return (
      <span className="ss-btn ss-btn--ghost" style={{ opacity: 0.4, pointerEvents: 'none' }}>
        {label}
      </span>
    );
  }
  return (
    <Link href={href} className="ss-btn ss-btn--ghost" prefetch={false}>
      {label}
    </Link>
  );
}

function ContractCard({ contract }: { contract: ContractSummary }) {
  const title = contract.display_name ?? contract.canonical_id;
  const reward =
    contract.reward_amount != null
      ? `${contract.reward_amount.toLocaleString()} ${contract.reward_currency ?? 'aUEC'}`
      : null;

  // `display_name` is intentionally non-unique — in-game contract names
  // genuinely repeat while the underlying contracts differ; canonical_id
  // is the real identity. These two lines surface whatever actually
  // differs (reward/item, patch/step count) so two same-named rows read
  // as distinct rather than as duplicates. Every field here is nullable
  // in practice, so each line is built from whichever parts are present
  // and disappears entirely when none are — never a stray `·` or `@`.
  const rewardLine = joinNoteParts([reward, contract.required_item]);
  const patchLine = joinNoteParts([
    contract.patch_version ? `patch ${contract.patch_version}` : null,
    contract.step_count != null
      ? `${contract.step_count} ${contract.step_count === 1 ? 'step' : 'steps'}`
      : null,
  ]);

  return (
    <Link
      href={`/contracts/${contract.canonical_id}` as Route}
      aria-label={title}
      prefetch={false}
      style={{ textDecoration: 'none', color: 'inherit' }}
    >
      <article
        className="hp-plane flat"
        style={{
          display: 'flex',
          flexDirection: 'column',
          gap: 4,
          height: '100%',
        }}
      >
        <span style={{ fontSize: 14, fontWeight: 600 }}>{title}</span>
        <span
          style={{
            display: 'flex',
            gap: 6,
            flexWrap: 'wrap',
            fontSize: 11,
            color: 'var(--fg-dim)',
          }}
        >
          {contract.contract_type && <span>{contract.contract_type}</span>}
          {contract.legal_status && (
            <span style={{ color: legalColor(contract.legal_status) }}>
              {contract.legal_status}
            </span>
          )}
        </span>
        {contract.issuer && (
          <span style={{ fontSize: 11, color: 'var(--fg-muted)' }}>
            <span style={{ color: 'var(--fg-dim)' }}>issuer: </span>
            {contract.issuer}
          </span>
        )}
        {contract.first_step_location && (
          <span className="hud-note">@ {contract.first_step_location}</span>
        )}
        {rewardLine && (
          <span className="hud-note" style={{ color: 'var(--accent)' }}>
            {rewardLine}
          </span>
        )}
        {patchLine && <span className="hud-note">{patchLine}</span>}
      </article>
    </Link>
  );
}

/** Join present segments with ' · ', dropping null/blank ones so an
 *  absent field never leaves a stray leading/trailing/doubled `·` — see
 *  `ContractCard`'s doc on why every part here is nullable in practice. */
function joinNoteParts(parts: Array<string | null | undefined>): string | null {
  const present = parts.filter((p): p is string => Boolean(p && p.trim()));
  return present.length > 0 ? present.join(' · ') : null;
}

function legalColor(status: string): string {
  const s = status.toLowerCase();
  // The beam's own tone. The flat fallback was the dark theme's literal and
  // stayed that colour on every calibration.
  if (s === 'illegal') return 'var(--bad)';
  if (s === 'legal') return 'var(--fg-muted)';
  return 'var(--fg-dim)';
}

function parsePositiveInt(raw: string | undefined): number {
  if (!raw) return 0;
  const n = Number.parseInt(raw, 10);
  if (!Number.isFinite(n) || n < 0) return 0;
  return n;
}

const rowStyle: React.CSSProperties = {
  display: 'flex',
  alignItems: 'center',
  flexWrap: 'wrap',
  gap: 6,
  marginTop: 12,
};

const rowLabelStyle: React.CSSProperties = {
  fontSize: 11,
  color: 'var(--fg-dim)',
  letterSpacing: '0.04em',
  minWidth: 52,
};
