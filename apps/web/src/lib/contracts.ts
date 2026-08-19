/**
 * Public contract browse data layer.
 *
 * Talks to the Rust server's public contract read surface
 * (`/api/contracts`, `/api/contracts/search`, `/api/contracts/{id}`) —
 * the receiving side of the sp-ingest push. All endpoints are public
 * (no auth), so these fetches carry no bearer token.
 *
 * Mirrors `lib/reference.ts`: fetch directly off `apiBase()` and honor
 * `STARSTATS_DISABLE_FETCH_CACHE` (set in the Playwright webServer env)
 * so Next 15's URL-keyed data cache doesn't leak one e2e scenario's
 * mock across the rest. Failures degrade to an empty listing rather
 * than throwing — the browse surface stays up when upstream hiccups.
 *
 * Note the base path is `/api/contracts`, NOT `/v1/...` — contracts are
 * a deliberately separate namespace from the tray-event API.
 */
import 'server-only';
import { apiBase } from '@/lib/api';
import type { components } from 'api-client-ts';

export type ContractSummary = components['schemas']['ContractSummary'];
export type ContractDetail = components['schemas']['ContractDetail'];
export type ContractListResponse = components['schemas']['ContractListResponse'];

/** Cache directive for contract fetches. `no-store` under the e2e
 *  env gate (so mock fixtures swap per scenario); a short revalidate
 *  window otherwise — contracts change only when sp-ingest pushes. */
function contractsCacheOpts(): RequestInit {
  if (process.env.STARSTATS_DISABLE_FETCH_CACHE === '1') {
    return { cache: 'no-store' };
  }
  return { next: { revalidate: 60 } } as RequestInit;
}

export interface ContractListParams {
  /** Free-text search term. Presence routes to `/search`. */
  q?: string;
  /** Location search alias. Presence routes to `/search`. */
  location?: string;
  /** Filter by contract type (case-insensitive). */
  type?: string;
  /** Filter by issuer (case-insensitive). */
  issuer?: string;
  /** Filter by faction (case-insensitive). The joint-strongest discriminator
   *  between same-named contracts, alongside `legalStatus`. */
  faction?: string;
  /** Filter by legal status (case-insensitive). */
  legalStatus?: string;
  /** Filter by gameplay loop (case-insensitive). */
  gameplayLoop?: string;
  limit?: number;
  offset?: number;
}

/**
 * Build the querystring for a contracts list/search request. Empty /
 * blank values are omitted, and `offset=0` is dropped so first-page
 * links stay clean. Pure — unit-tested in `contracts.test.ts`.
 */
export function buildContractsQuery(params: ContractListParams): string {
  const qs = new URLSearchParams();
  const set = (key: string, value: string | number | undefined): void => {
    if (value === undefined) return;
    const s = String(value).trim();
    if (s !== '') qs.set(key, s);
  };
  set('q', params.q);
  set('location', params.location);
  set('type', params.type);
  set('issuer', params.issuer);
  set('faction', params.faction);
  set('legal_status', params.legalStatus);
  set('gameplay_loop', params.gameplayLoop);
  if (params.limit !== undefined) set('limit', params.limit);
  if (params.offset !== undefined && params.offset > 0) set('offset', params.offset);
  return qs.toString();
}

/** True when the request should hit the `/search` endpoint (a free-text
 *  or location term is present) rather than the plain list. */
export function isSearchRequest(params: ContractListParams): boolean {
  return Boolean(params.q?.trim() || params.location?.trim());
}

const EMPTY_LISTING: ContractListResponse = { contracts: [], next_offset: null };

/**
 * Fetch a page of contracts. Routes to `/api/contracts/search` when a
 * `q`/`location` term is present, else `/api/contracts`. Degrades to an
 * empty listing (logged) on any non-2xx or network error.
 */
/** Contracts referencing a knowledge-base entity, for the KB entity
 *  page's Contracts section.
 *
 *  Returns `[]` on any failure so the caller's guard is a length check:
 *  a contracts hiccup must never break a KB page, matching how `/kb`
 *  already guards its contract count. */
export async function listContractsByEntity(
  category: string,
  slug: string,
): Promise<ContractSummary[]> {
  const qs = new URLSearchParams({ category, slug }).toString();
  const url = `${apiBase()}/api/contracts/by-entity?${qs}`;
  try {
    const resp = await fetch(url, { method: 'GET', ...contractsCacheOpts() });
    if (!resp.ok) {
      console.error(`contracts by-entity fetch failed status=${resp.status}`);
      return [];
    }
    const body = (await resp.json()) as ContractListResponse;
    return body.contracts ?? [];
  } catch (e) {
    console.error('contracts by-entity fetch threw', e);
    return [];
  }
}

/** Resolve contract names to catalogue entries, batched.
 *
 *  One request for every name on the page: `/me/contracts` renders many
 *  run cards and a request per card would burst the per-IP governor the
 *  way the KB prefetch storm did. */
export async function resolveContractNames(
  names: string[],
): Promise<Map<string, NameResolution>> {
  const distinct = [...new Set(names.map((n) => n.trim()).filter(Boolean))].slice(0, 100);
  if (distinct.length === 0) return new Map();
  const qs = new URLSearchParams();
  for (const n of distinct) qs.append('name', n);
  try {
    const resp = await fetch(`${apiBase()}/api/contracts/resolve?${qs}`, {
      method: 'GET',
      ...contractsCacheOpts(),
    });
    if (!resp.ok) {
      console.error(`contracts resolve fetch failed status=${resp.status}`);
      return new Map();
    }
    const body = (await resp.json()) as { resolved?: NameResolution[] };
    return new Map((body.resolved ?? []).map((r) => [r.name, r]));
  } catch (e) {
    console.error('contracts resolve fetch threw', e);
    return new Map();
  }
}

export interface NameResolution {
  name: string;
  match_count: number;
  canonical_id: string | null;
}

/** Where a rendered contract name should link, or `null` for plain text.
 *
 *  Three tiers, and the middle one is the point: `display_name` is
 *  non-unique by design, so an ambiguous name goes to the filtered
 *  candidate list — which carries the disambiguation row a person needs
 *  — rather than to an arbitrary winner. A confident wrong link is
 *  worse than no link. */
export function contractNameHref(name: string, r?: NameResolution): string | null {
  if (!r || r.match_count === 0) return null;
  if (r.match_count === 1 && r.canonical_id) return `/contracts/${r.canonical_id}`;
  return `/contracts?${new URLSearchParams({ q: name }).toString()}`;
}

/** Normalize a name the way the server does, for map lookups. */
export function normalizeContractName(name: string): string {
  return name.trim().replace(/\s+/g, ' ').toLowerCase();
}

export async function listContracts(
  params: ContractListParams,
): Promise<ContractListResponse> {
  const path = isSearchRequest(params) ? '/api/contracts/search' : '/api/contracts';
  const qs = buildContractsQuery(params);
  const url = `${apiBase()}${path}${qs ? `?${qs}` : ''}`;
  try {
    const resp = await fetch(url, { method: 'GET', ...contractsCacheOpts() });
    if (!resp.ok) {
      console.error(`contracts list fetch failed call=${path} status=${resp.status}`);
      return EMPTY_LISTING;
    }
    return (await resp.json()) as ContractListResponse;
  } catch (e) {
    console.error('contracts list fetch threw', e);
    return EMPTY_LISTING;
  }
}

/** Discriminated fetch outcome so the detail page can 404 vs. throw. */
export type ContractDetailOutcome =
  | { kind: 'ok'; contract: ContractDetail }
  | { kind: 'not_found' }
  | { kind: 'error'; reason: string };

/**
 * Fetch one contract by canonical id. `not_found` collapses to the 404
 * page; a transient `error` is thrown so the Next error boundary can
 * offer a retry rather than a misleading permanent 404 (same posture
 * as `getEntityDetail`).
 */
export async function getContractDetail(
  canonicalId: string,
): Promise<ContractDetailOutcome> {
  const url = `${apiBase()}/api/contracts/${encodeURIComponent(canonicalId)}`;
  try {
    const resp = await fetch(url, { method: 'GET', ...contractsCacheOpts() });
    if (resp.status === 404) return { kind: 'not_found' };
    if (!resp.ok) return { kind: 'error', reason: `status ${resp.status}` };
    const contract = (await resp.json()) as ContractDetail;
    return { kind: 'ok', contract };
  } catch (e) {
    return {
      kind: 'error',
      reason: e instanceof Error ? e.message : 'fetch failed',
    };
  }
}

/**
 * Fetch every contract by paging through the list endpoint. Used for
 * facet vocabularies + the KB landing count. Small dataset today; the
 * 50-iteration cap (×200 = 10k) is a runaway guard. Degrades to
 * whatever it collected on error.
 */
export async function listAllContracts(): Promise<ContractSummary[]> {
  const acc: ContractSummary[] = [];
  let offset = 0;
  for (let i = 0; i < 50; i++) {
    const page = await listContracts({ limit: 200, offset });
    acc.push(...page.contracts);
    if (page.next_offset == null) break;
    offset = page.next_offset;
  }
  return acc;
}

/** Facet columns the contracts browse can filter on. */
export type ContractFacetKey =
  | 'contract_type'
  | 'issuer'
  | 'faction'
  | 'legal_status'
  | 'gameplay_loop';

/** Sort orders offered on the contracts browse. */
export type ContractSortKey = 'updated' | 'name' | 'reward';

/** Distinct non-empty values for a facet, case-folded to dedupe and
 *  sorted for stable chip rendering. Pure. */
export function distinctFacetValues(
  rows: ContractSummary[],
  key: ContractFacetKey,
): string[] {
  const seen = new Map<string, string>(); // lower -> first-seen original casing
  for (const r of rows) {
    const v = r[key];
    if (v && v.trim()) {
      const k = v.toLowerCase();
      if (!seen.has(k)) seen.set(k, v);
    }
  }
  return [...seen.values()].sort((a, b) => a.localeCompare(b));
}

export interface ContractFacetFilters {
  type?: string;
  issuer?: string;
  faction?: string;
  legalStatus?: string;
  gameplayLoop?: string;
}

/** Apply the selected facet filters (case-insensitive exact match).
 *  Absent/blank filters match everything. Pure. */
export function applyContractFacets(
  rows: ContractSummary[],
  f: ContractFacetFilters,
): ContractSummary[] {
  const eq = (a: string | null | undefined, b?: string): boolean =>
    !b || !b.trim() || (a ?? '').toLowerCase() === b.toLowerCase();
  return rows.filter(
    (r) =>
      eq(r.contract_type, f.type) &&
      eq(r.issuer, f.issuer) &&
      eq(r.faction, f.faction) &&
      eq(r.legal_status, f.legalStatus) &&
      eq(r.gameplay_loop, f.gameplayLoop),
  );
}

/** Sort a copy of the rows by the chosen key. `updated` (default) is
 *  newest-first on the ISO `updated_at` (lexical sort is valid for
 *  rfc3339); `reward` is amount high→low (nulls last); `name` is
 *  A–Z by display name. Pure. */
export function sortContractSummaries(
  rows: ContractSummary[],
  key: ContractSortKey,
): ContractSummary[] {
  const out = [...rows];
  if (key === 'name') {
    out.sort((a, b) =>
      (a.display_name ?? a.canonical_id).localeCompare(b.display_name ?? b.canonical_id),
    );
  } else if (key === 'reward') {
    out.sort((a, b) => (b.reward_amount ?? -1) - (a.reward_amount ?? -1));
  } else {
    out.sort((a, b) => (b.updated_at || '').localeCompare(a.updated_at || ''));
  }
  return out;
}

/** Format a reward as a compact display string, e.g. "8,500 aUEC
 *  (+1,500)". Returns null when there's no amount to show. Accepts a
 *  loose shape so both the strict generated `reward` and partial test
 *  inputs work. Pure — unit-tested. */
export function formatReward(
  reward:
    | { amount?: number | null; currency?: string | null; bonus_amount?: number | null }
    | null
    | undefined,
): string | null {
  if (!reward || reward.amount == null) return null;
  const currency = reward.currency ?? 'aUEC';
  const base = `${reward.amount.toLocaleString()} ${currency}`;
  if (reward.bonus_amount != null && reward.bonus_amount > 0) {
    return `${base} (+${reward.bonus_amount.toLocaleString()})`;
  }
  return base;
}

/** A non-aUEC award, ready to render. */
export interface AdditionalRewardLine {
  /** `"14 MG Scrip"`, or bare `"MG Scrip"` when no count was stated. */
  text: string;
  /** Verbatim sentence the award was read from, or `null`. */
  note: string | null;
}

/** Format `reward.additional` — the non-aUEC awards (MG Scrip, Council
 *  Scrip, ...) that the sender reads out of the DETAILS prose rather
 *  than the Reward header.
 *
 *  Unlike `formatReward`, a missing count is NOT a missing reward: the
 *  prose frequently names an award without a number, and those entries
 *  are often the only evidence a contract pays anything but aUEC. An
 *  entry with no `unit` is dropped instead — a bare "14" names nothing.
 *
 *  Takes a loose shape so both the strict generated `reward` and
 *  partial test fixtures type-check. */
export function formatAdditionalRewards(
  reward:
    | {
        additional?:
          | ReadonlyArray<{
              amount?: number | null;
              unit?: string | null;
              note?: string | null;
            }>
          | null;
      }
    | null
    | undefined,
): AdditionalRewardLine[] {
  if (!reward || !Array.isArray(reward.additional)) return [];
  const lines: AdditionalRewardLine[] = [];
  for (const entry of reward.additional) {
    const unit = entry?.unit?.trim();
    if (!unit) continue;
    const text = entry.amount == null ? unit : `${entry.amount.toLocaleString()} ${unit}`;
    lines.push({ text, note: entry.note ?? null });
  }
  return lines;
}

/** Presentation for the mission-timer indicator. */
export interface MissionTimerBadge {
  label: string;
  /** CSS colour token for the badge border/text. */
  tone: string;
}

/**
 * Badge for a contract that has a mission timer (a limited window to
 * accept or complete). Returns a badge only when `has_time_limit` is
 * explicitly `true`; `false` (no limit) and `null`/absent (unknown) both
 * return null so the UI shows no badge. Accepts a loose shape so the
 * strict generated `timeframe` and partial test inputs both work. Pure —
 * unit-tested. */
export function missionTimerBadge(
  timeframe: { has_time_limit?: boolean | null } | null | undefined,
): MissionTimerBadge | null {
  if (timeframe?.has_time_limit === true) {
    return { label: 'Timed', tone: 'var(--warn, #f5a623)' };
  }
  return null;
}
