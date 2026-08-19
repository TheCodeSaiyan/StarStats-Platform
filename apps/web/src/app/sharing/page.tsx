/**
 * Sharing — outbound + inbound stats-record sharing surface.
 *
 * Promoted from a buried section inside /settings#sharing into its
 * own surface so:
 *  - users can discover sharing without spelunking through settings,
 *  - the inbound side ("who has shared with you") has a home, and
 *  - per-profile CTAs ("Manage sharing", "Share back") can deep-link
 *    to a single canonical place rather than a settings anchor.
 *
 * Backend contracts:
 *  - GET  /v1/me/visibility                — public toggle state
 *  - POST /v1/me/visibility {public:bool}  — flip the toggle
 *  - GET  /v1/me/shares                    — outbound (user + org)
 *  - POST /v1/me/share {recipient_handle}  — grant to handle
 *  - POST /v1/me/share/org {org_slug}      — grant to org
 *  - DEL  /v1/me/share/:recipient_handle   — revoke handle
 *  - DEL  /v1/me/share/org/:slug           — revoke org
 *  - GET  /v1/me/shared-with-me            — inbound (new in this wave)
 *
 * SpiceDB unavailability degrades the UI but never blocks the page —
 * the user can still navigate away. RSI-unverified callers can read
 * state (degraded mode warning) but mutation handlers return 403.
 */

import Link from 'next/link';
import type { Route } from 'next';
import { redirect } from 'next/navigation';
import {
  ApiCallError,
  addShare,
  getProfileViews,
  getVisibility,
  listOrgs,
  listShares,
  listSharedWithMe,
  removeShare,
  setVisibility,
  shareWithOrg,
  unshareWithOrg,
  type ListOrgsResponse,
  type ListSharedWithMeResponse,
  type ListSharesResponse,
  type ProfileViewStats,
  type ShareScope,
  type SharedWithMeEntry,
  type VisibilityResponse,
} from '@/lib/api';
import { localInputToUtcIso } from '@/lib/expiry';
import { logger } from '@/lib/logger';
import { getSession } from '@/lib/session';
import {
  bulkResetScopeAction,
  bulkRevokeExpiredAction,
  reportShareAction,
} from './actions';
import { ExpiryField } from './ExpiryField';
import { ScopePresets } from './_components/ScopePresets';
import { PreviewButton } from './_components/PreviewButton';
import { InstrumentStrip } from '@/components/hud/InstrumentStrip';

export const metadata = { title: "Sharing" };

interface SearchParams {
  status?: string;
  error?: string;
  /**
   * Counter carried by bulk-op redirects so the success banner can
   * say "Revoked 3 expired shares" instead of "Revoked." Audit
   * v2.1 §A3.
   */
  n?: string;
  /** Pre-populate the add-handle field — set by per-profile "Share back" CTA. */
  handle?: string;
  /**
   * Pre-populate the optional expiry field for in-place edit. Format
   * is the `<input type="datetime-local">` shape (`YYYY-MM-DDTHH:MM`
   * in local time) — what the browser already submits, and what the
   * edit-Link writes back into the URL.
   */
  expires?: string;
  /** Pre-populate the note field for in-place edit. */
  note?: string;
}

const pageStyle: React.CSSProperties = {
  display: 'flex',
  flexDirection: 'column',
  gap: 14,
  maxWidth: 960,
  margin: '0 auto',
  padding: '8px 0 60px',
};

const cardHeaderStyle: React.CSSProperties = { padding: '13px 16px 0' };
const cardBodyStyle: React.CSSProperties = {
  padding: '11px 16px 14px',
  display: 'flex',
  flexDirection: 'column',
  gap: 10,
};
const cardTitleStyle: React.CSSProperties = {
  margin: 0,
  fontSize: 15,
  fontWeight: 600,
  letterSpacing: '-0.01em',
};
const mutedStyle: React.CSSProperties = {
  margin: 0,
  color: 'var(--fg-muted)',
  fontSize: 13,
};
const sharePillStyle: React.CSSProperties = {
  display: 'flex',
  alignItems: 'center',
  justifyContent: 'space-between',
  gap: 12,
  padding: '8px 12px',
  background: 'var(--bg-elev)',
  border: '1px solid var(--border)',
  borderRadius: 'var(--r-sm)',
};
const formRowStyle: React.CSSProperties = {
  display: 'flex',
  gap: 8,
  alignItems: 'center',
  flexWrap: 'wrap',
};

/**
 * Status banner copy. `text` may be a function when the message
 * needs to interpolate a counter (bulk-op results carry one in the
 * `n` query param). Static-text entries use the literal shape.
 */
const STATUS_MESSAGES: Record<
  string,
  { text: string | ((n: number) => string); tone: 'ok' | 'danger' }
> = {
  visibility_public: { text: 'Profile is now public.', tone: 'ok' },
  visibility_private: { text: 'Profile is now private.', tone: 'ok' },
  // Piece 4 — `/discover` listing sub-toggle. Distinct from the
  // public/private status so a user who flips only the listing
  // doesn't see "Profile is now public" misleadingly re-rendered.
  listing_hidden: {
    text: 'Hidden from the public profile listings.',
    tone: 'ok',
  },
  listing_shown: {
    text: 'Listed on the public profile listings.',
    tone: 'ok',
  },
  share_added: { text: 'Share granted.', tone: 'ok' },
  share_revoked: { text: 'Share revoked.', tone: 'ok' },
  report_filed: {
    text: 'Report submitted — our moderators will review it. Thanks.',
    tone: 'ok',
  },
  org_share_added: { text: 'Org share granted.', tone: 'ok' },
  org_share_revoked: { text: 'Org share revoked.', tone: 'ok' },
  // Audit v2.1 §A3 — bulk-op outcomes.
  bulk_revoked: {
    text: (n) =>
      n === 0
        ? 'No expired shares were left to revoke.'
        : `Revoked ${n} expired share${n === 1 ? '' : 's'}.`,
    tone: 'ok',
  },
  bulk_scope_reset: {
    text: (n) =>
      n === 0
        ? 'No active outbound shares to update.'
        : `Reset scope on ${n} share${n === 1 ? '' : 's'}.`,
    tone: 'ok',
  },
  bulk_revoke_failed: {
    text: "Couldn't load shares to revoke. Try again shortly.",
    tone: 'danger',
  },
  bulk_scope_reset_failed: {
    text: "Couldn't reset scopes. Try again shortly.",
    tone: 'danger',
  },
};

const ERROR_MESSAGES: Record<string, string> = {
  rsi_handle_not_verified:
    'Verify your RSI handle in Settings before granting shares.',
  report_invalid: 'Pick a reason before submitting the report.',
  report_failed: "Couldn't submit the report. Try again shortly.",
  recipient_not_found: 'No StarStats account exists for that handle.',
  org_not_found: 'No org exists with that slug.',
  invalid_recipient_handle: 'Handle looks invalid — letters, digits, dashes only.',
  invalid_org_slug: 'Org slug looks invalid.',
  cannot_share_with_self: "You can't share your stats with yourself.",
  expires_at_in_past: 'Expiry must be in the future.',
  note_too_long: 'Note is too long (max 280 characters).',
  invalid_scope_kind: 'Pick a valid scope kind.',
  invalid_scope_window: 'Scope window must be between 1 and 90 days.',
  invalid_scope_tabs: 'One of the selected tabs is unknown.',
  invalid_scope_types: 'Event-type filter contains invalid entries.',
  spicedb_unavailable:
    'The authorisation service is offline. Try again shortly.',
  unexpected: 'Something went wrong. Try again.',
};

/** Closed vocabulary mirroring `ALLOWED_SCOPE_TABS` in the Rust
 *  validator. Centralising both lists in the page makes it cheap to
 *  add a new tab — bump both sides and the picker just works. */
const SCOPE_TAB_OPTIONS: ReadonlyArray<{ value: string; label: string }> = [
  { value: 'location', label: 'Location' },
  { value: 'travel', label: 'Travel' },
  { value: 'combat', label: 'Combat' },
  { value: 'loadout', label: 'Loadout' },
  { value: 'stability', label: 'Stability' },
  { value: 'commerce', label: 'Commerce' },
];

/** Format a timestamp as a short relative string like "3d ago" or
 *  "just now". Returns null for missing input so the caller can
 *  conditionally render. */
function formatRelativePast(iso: string | null | undefined): string | null {
  if (!iso) return null;
  const ts = new Date(iso);
  if (Number.isNaN(ts.getTime())) return null;
  const diffMs = Date.now() - ts.getTime();
  if (diffMs < 0) return 'just now';
  const min = Math.round(diffMs / 60_000);
  if (min < 1) return 'just now';
  if (min < 60) return `${min}m ago`;
  const hr = Math.round(min / 60);
  if (hr < 24) return `${hr}h ago`;
  const day = Math.round(hr / 24);
  return `${day}d ago`;
}

/**
 * Build the per-pill "Edit" URL. Round-trips the share's current
 * expiry + note through the URL so the existing add-share form can
 * pre-fill them; submitting that form re-POSTs to /v1/me/share which
 * upserts the metadata (set + clear are both supported now). The
 * expiry is serialised as the FULL UTC ISO instant — `<ExpiryField>`
 * localizes it into the datetime-local input using the browser's
 * timezone offset, so the prefill shows the same wall-clock the user
 * originally picked rather than drifting by the UTC offset each edit.
 */
function buildEditHref(
  recipientHandle: string,
  expiresAt: string | null | undefined,
  note: string | null | undefined,
): string {
  const qs = new URLSearchParams();
  qs.set('handle', recipientHandle);
  if (expiresAt) {
    const dt = new Date(expiresAt);
    if (!Number.isNaN(dt.getTime())) {
      // Emit the honest UTC instant; ExpiryField converts it to a
      // local wall-clock on the client for display, keeping the
      // round-trip symmetric with localInputToUtcIso on submit.
      qs.set('expires', dt.toISOString());
    }
  }
  if (note) qs.set('note', note);
  return `/sharing?${qs.toString()}#share-editor`;
}

/** Format an ISO timestamp as "in 3d" / "expired" / "in 2h" for the
 *  share pills. Returns null when no expiry was set. */
function formatExpiry(iso: string | null | undefined): string | null {
  if (!iso) return null;
  const ts = new Date(iso);
  if (Number.isNaN(ts.getTime())) return null;
  const now = Date.now();
  const diffMs = ts.getTime() - now;
  if (diffMs <= 0) return 'expired';
  const diffMin = Math.round(diffMs / 60_000);
  if (diffMin < 60) return `in ${diffMin}m`;
  const diffHr = Math.round(diffMin / 60);
  if (diffHr < 24) return `in ${diffHr}h`;
  const diffDay = Math.round(diffHr / 24);
  return `in ${diffDay}d`;
}

export default async function SharingPage(props: {
  searchParams: Promise<SearchParams>;
}) {
  const session = await getSession();
  if (!session) redirect('/auth/login?next=/sharing');

  const params = await props.searchParams;
  const status = params.status;
  const errorCode = params.error;
  const prefilledHandle = (params.handle ?? '').trim();
  const prefilledExpires = (params.expires ?? '').trim();
  const prefilledNote = (params.note ?? '').trim();
  // "Edit mode" = any of the prefill fields are set. Switches the
  // form's title/button copy from "grant" to "save changes" so the
  // user understands they're updating an existing row.
  const isEditing = prefilledHandle !== '';

  // Load the four parallel data sources. Per-call settling so a
  // single endpoint hiccup doesn't take down the whole page; the
  // surviving sections still render with whatever data came back.
  // SpiceDB 503 on any call flips to a clear "temporarily
  // unavailable" banner; only an all-fail (every call rejected with
  // something other than 401/503) falls through to the generic
  // error fallback. 401 on any call short-circuits to login.
  let visibility: VisibilityResponse | null = null;
  let shares: ListSharesResponse | null = null;
  let inbound: ListSharedWithMeResponse | null = null;
  let myOrgs: ListOrgsResponse | null = null;
  let profileViews: ProfileViewStats | null = null;
  let degraded: 'spicedb_unavailable' | 'unknown' | null = null;

  const [visRes, sharesRes, inboundRes, orgsRes, viewsRes] =
    await Promise.allSettled([
      getVisibility(session.token),
      listShares(session.token),
      listSharedWithMe(session.token),
      listOrgs(session.token),
      // Profile-view counters power the "Profile views" card. We fetch
      // alongside the others but treat a rejection as soft-fail — the
      // card just won't render. The visibility / shares paths are the
      // load-bearing ones.
      getProfileViews(session.token, { days: 30 }),
    ]);

  // 401 on any call -> re-auth. Look across all 5 results so a
  // refresh-token failure on one of them still kicks us to login
  // instead of half-rendering as anonymous.
  for (const r of [visRes, sharesRes, inboundRes, orgsRes, viewsRes]) {
    if (
      r.status === 'rejected' &&
      r.reason instanceof ApiCallError &&
      r.reason.status === 401
    ) {
      redirect('/auth/login?next=/sharing');
    }
  }

  // SpiceDB 503 on any call -> show the temporarily-unavailable
  // banner. Don't try to render partial state in that case — the
  // page leans on ReBAC for almost every section. (profile-views
  // doesn't depend on SpiceDB, so it's not in the 503 set, but we
  // include it anyway for symmetry — its 503 would still be a
  // server-side outage worth surfacing.)
  for (const r of [visRes, sharesRes, inboundRes, orgsRes, viewsRes]) {
    if (
      r.status === 'rejected' &&
      r.reason instanceof ApiCallError &&
      r.reason.status === 503
    ) {
      degraded = 'spicedb_unavailable';
      break;
    }
  }

  if (degraded === null) {
    if (visRes.status === 'fulfilled') visibility = visRes.value;
    if (sharesRes.status === 'fulfilled') shares = sharesRes.value;
    if (inboundRes.status === 'fulfilled') inbound = inboundRes.value;
    if (orgsRes.status === 'fulfilled') myOrgs = orgsRes.value;
    if (viewsRes.status === 'fulfilled') profileViews = viewsRes.value;

    // Log every rejection individually so server logs name the
    // failing endpoint instead of swallowing it under one umbrella.
    // Status is captured when it's an ApiCallError so the failure
    // mode (4xx vs 5xx vs network) is visible without a full stack.
    for (const [label, r] of [
      ['getVisibility', visRes],
      ['listShares', sharesRes],
      ['listSharedWithMe', inboundRes],
      ['listOrgs', orgsRes],
      ['getProfileViews', viewsRes],
    ] as const) {
      if (r.status === 'rejected') {
        const status =
          r.reason instanceof ApiCallError ? r.reason.status : undefined;
        logger.error(
          { err: r.reason, call: label, status },
          'sharing data fetch rejected',
        );
      }
    }

    // Fall back to the generic error fallback only when EVERY load-
    // bearing call failed. The profile-views call is NOT load-bearing
    // (the card is decorative) so it's excluded from the all-fail
    // check — a stale endpoint shouldn't blank out sharing.
    const allFailed =
      visRes.status === 'rejected' &&
      sharesRes.status === 'rejected' &&
      inboundRes.status === 'rejected' &&
      orgsRes.status === 'rejected';
    if (allFailed) degraded = 'unknown';
  }

  // -- Server actions --------------------------------------------------

  async function visibilityAction(formData: FormData) {
    'use server';
    const s = await getSession();
    if (!s) redirect('/auth/login?next=/sharing');
    const wantPublic = String(formData.get('public') ?? 'false') === 'true';
    // Hold the server's response so the status chip echoes what
    // actually landed — not the user's intent. A backend that 200-OKs
    // a no-op (e.g. SpiceDB read failing after a successful write, or
    // a future read-only-mode shim) would otherwise produce a "Profile
    // is now public." chip on a page that still says "Private",
    // gaslighting the user. Surfaced 2026-05-21 via the wildcard-
    // CheckPermission InvalidArgument regression.
    let response!: VisibilityResponse;
    try {
      response = await setVisibility(s.token, wantPublic);
    } catch (e) {
      if (e instanceof ApiCallError && e.status === 401)
        redirect('/auth/login?next=/sharing');
      if (e instanceof ApiCallError && e.status === 403)
        redirect('/sharing?error=rsi_handle_not_verified');
      if (e instanceof ApiCallError && e.status === 503)
        redirect('/sharing?error=spicedb_unavailable');
      logger.error({ err: e }, 'set visibility failed');
      redirect('/sharing?error=unexpected');
    }
    redirect(
      `/sharing?status=visibility_${response.public ? 'public' : 'private'}`,
    );
  }

  // Piece 4 — listing_opt_out sub-toggle. Lives behind the same
  // POST /v1/me/visibility endpoint so the server can audit both
  // flips together. The form posts a single `listing_opt_out` field;
  // the `public` flag is omitted as a wire-level signal that we want
  // to leave the SpiceDB-side value untouched. The server allows
  // this via the optional `listing_opt_out` field on
  // `VisibilityRequest` — clients that only send `{"public": ...}`
  // (the existing /sharing visibility action above) keep working
  // unchanged.
  async function listingOptOutAction(formData: FormData) {
    'use server';
    const s = await getSession();
    if (!s) redirect('/auth/login?next=/sharing');
    // The hidden input writes the DESIRED next value (the inverse of
    // the current state) so a click on either label flips the toggle.
    const wantOptOut =
      String(formData.get('listing_opt_out') ?? 'false') === 'true';
    // Read the current public state so we forward it through; the
    // server treats the `public` field as authoritative. A bug-fix
    // expectation: the user is on the page because they've already
    // toggled public ON — but in a race we still ship the truth from
    // the current render.
    const currentPublic = visibility?.public === true;
    // Hold the server's response so the chip echoes what actually landed,
    // not the user's intent — mirrors visibilityAction. A 200-OK no-op
    // would otherwise show a "listing hidden" chip on a page that never
    // changed, gaslighting the user. `listing_opt_out` is always echoed. (M-W1)
    let response!: VisibilityResponse;
    try {
      response = await setVisibility(s.token, currentPublic, wantOptOut);
    } catch (e) {
      if (e instanceof ApiCallError && e.status === 401)
        redirect('/auth/login?next=/sharing');
      if (e instanceof ApiCallError && e.status === 403)
        redirect('/sharing?error=rsi_handle_not_verified');
      if (e instanceof ApiCallError && e.status === 503)
        redirect('/sharing?error=spicedb_unavailable');
      logger.error({ err: e }, 'set listing_opt_out failed');
      redirect('/sharing?error=unexpected');
    }
    redirect(
      `/sharing?status=listing_${response.listing_opt_out ? 'hidden' : 'shown'}`,
    );
  }

  async function addShareAction(formData: FormData) {
    'use server';
    const s = await getSession();
    if (!s) redirect('/auth/login?next=/sharing');
    const recipient = String(formData.get('recipient_handle') ?? '').trim();
    if (recipient === '') redirect('/sharing?error=invalid_recipient_handle');
    // Optional expiry comes from an <input type="datetime-local">,
    // which returns a naive local string like "2026-06-01T10:00" with
    // NO timezone. `<ExpiryField>` ships the browser's offset alongside
    // it in a hidden field; convert exactly once to a UTC instant so the
    // server can compare against UTC. A missing/blank offset degrades to
    // UTC (offset 0) rather than guessing the server's zone. Empty = no
    // expiry. See `@/lib/expiry`.
    const expiresLocal = String(formData.get('expires_at_local') ?? '').trim();
    const tzOffsetRaw = String(formData.get('tz_offset_minutes') ?? '').trim();
    const parsedOffset = Number(tzOffsetRaw);
    const offsetMinutes = Number.isFinite(parsedOffset) ? parsedOffset : 0;
    const expiresAt = localInputToUtcIso(expiresLocal, offsetMinutes);
    const noteRaw = String(formData.get('note') ?? '').trim();
    const note = noteRaw === '' ? null : noteRaw;
    // Build the scope payload from the picker. `kind="full"` (or
    // absent) is the legacy default and skips the wire field — the
    // server normalises kind=full back to NULL anyway, but keeping
    // the body minimal makes the audit-log payload easier to read.
    const scopeKind = String(formData.get('scope_kind') ?? 'full').trim();
    let scope: ShareScope | null = null;
    if (scopeKind && scopeKind !== 'full') {
      const tabs = formData.getAll('scope_tabs').map(String).filter(Boolean);
      const windowDaysRaw = String(formData.get('scope_window_days') ?? '').trim();
      const windowDays = windowDaysRaw === '' ? null : Number(windowDaysRaw);
      const denyRaw = String(formData.get('scope_deny_event_types') ?? '').trim();
      const allowRaw = String(formData.get('scope_allow_event_types') ?? '').trim();
      // Comma-separated, lowercased, deduped. Empty list -> null so
      // we don't ship `[]` and trigger a "list too long" code-path
      // false positive on the server.
      const parseTypeList = (raw: string): string[] | null => {
        const parts = raw
          .split(',')
          .map((s) => s.trim().toLowerCase())
          .filter((s) => s.length > 0);
        return parts.length === 0 ? null : Array.from(new Set(parts));
      };
      scope = {
        kind: scopeKind,
        tabs: scopeKind === 'tabs' && tabs.length > 0 ? tabs : null,
        window_days:
          windowDays !== null && Number.isFinite(windowDays) ? windowDays : null,
        allow_event_types: parseTypeList(allowRaw),
        deny_event_types: parseTypeList(denyRaw),
      };
    }
    try {
      await addShare(s.token, recipient, { expiresAt, note, scope });
    } catch (e) {
      if (e instanceof ApiCallError) {
        if (e.status === 401) redirect('/auth/login?next=/sharing');
        if (e.status === 403) redirect('/sharing?error=rsi_handle_not_verified');
        if (e.status === 404) redirect('/sharing?error=recipient_not_found');
        if (e.status === 400)
          redirect(`/sharing?error=${encodeURIComponent(e.body.error)}`);
        if (e.status === 503) redirect('/sharing?error=spicedb_unavailable');
      }
      logger.error({ err: e }, 'add share failed');
      redirect('/sharing?error=unexpected');
    }
    redirect('/sharing?status=share_added');
  }

  async function revokeShareAction(formData: FormData) {
    'use server';
    const s = await getSession();
    if (!s) redirect('/auth/login?next=/sharing');
    const recipient = String(formData.get('recipient_handle') ?? '').trim();
    if (recipient === '') redirect('/sharing?error=invalid_recipient_handle');
    try {
      await removeShare(s.token, recipient);
    } catch (e) {
      if (e instanceof ApiCallError) {
        if (e.status === 401) redirect('/auth/login?next=/sharing');
        if (e.status === 503) redirect('/sharing?error=spicedb_unavailable');
      }
      logger.error({ err: e }, 'remove share failed');
      redirect('/sharing?error=unexpected');
    }
    redirect('/sharing?status=share_revoked');
  }

  async function shareOrgAction(formData: FormData) {
    'use server';
    const s = await getSession();
    if (!s) redirect('/auth/login?next=/sharing');
    const slug = String(formData.get('org_slug') ?? '').trim();
    if (slug === '') redirect('/sharing?error=invalid_org_slug');
    try {
      await shareWithOrg(s.token, slug);
    } catch (e) {
      if (e instanceof ApiCallError) {
        if (e.status === 401) redirect('/auth/login?next=/sharing');
        if (e.status === 403) redirect('/sharing?error=rsi_handle_not_verified');
        if (e.status === 404) redirect('/sharing?error=org_not_found');
        if (e.status === 400)
          redirect(`/sharing?error=${encodeURIComponent(e.body.error)}`);
        if (e.status === 503) redirect('/sharing?error=spicedb_unavailable');
      }
      logger.error({ err: e }, 'share with org failed');
      redirect('/sharing?error=unexpected');
    }
    redirect('/sharing?status=org_share_added');
  }

  async function revokeOrgShareAction(formData: FormData) {
    'use server';
    const s = await getSession();
    if (!s) redirect('/auth/login?next=/sharing');
    const slug = String(formData.get('org_slug') ?? '').trim();
    if (slug === '') redirect('/sharing?error=invalid_org_slug');
    try {
      await unshareWithOrg(s.token, slug);
    } catch (e) {
      if (e instanceof ApiCallError) {
        if (e.status === 401) redirect('/auth/login?next=/sharing');
        if (e.status === 503) redirect('/sharing?error=spicedb_unavailable');
      }
      logger.error({ err: e }, 'remove org share failed');
      redirect('/sharing?error=unexpected');
    }
    redirect('/sharing?status=org_share_revoked');
  }

  // -- Render ----------------------------------------------------------

  return (
    // role="main" over <main> element — global 720px column avoidance (M-W9).
    <div role="main" className="ss-screen-enter" style={pageStyle}>
      <header>
        <InstrumentStrip
          title={<h1 className="hud-tile__title" style={{ margin: 0, fontSize: 18 }}>Sharing</h1>}
          context="Community · who can see your manifest"
        />
        <p
          style={{
            margin: '6px 0 0',
            color: 'var(--fg-muted)',
            fontSize: 14,
          }}
        >
          Make your profile public, grant view-access to specific handles
          and orgs, and see who has shared their manifest with you.
        </p>
      </header>

      {status && STATUS_MESSAGES[status] && (
        <div
          role="status"
          className={`ss-badge ${
            STATUS_MESSAGES[status].tone === 'ok' ? 'ss-badge--ok' : ''
          }`}
          style={{ alignSelf: 'flex-start' }}
        >
          {typeof STATUS_MESSAGES[status].text === 'function'
            ? STATUS_MESSAGES[status].text(
                Number.parseInt(params.n ?? '0', 10) || 0,
              )
            : STATUS_MESSAGES[status].text}
        </div>
      )}
      {errorCode && (
        <div
          role="alert"
          className="ss-badge"
          style={{
            alignSelf: 'flex-start',
            borderColor: 'var(--danger)',
            color: 'var(--danger)',
          }}
        >
          {ERROR_MESSAGES[errorCode] ?? errorCode}
        </div>
      )}

      {degraded === 'spicedb_unavailable' ? (
        <section className="ss-card">
          <div style={cardBodyStyle}>
            <p style={mutedStyle}>
              Sharing is temporarily unavailable — the authorisation
              service is offline. Try again shortly.
            </p>
          </div>
        </section>
      ) : degraded === 'unknown' ? (
        <section className="ss-card">
          <div style={cardBodyStyle}>
            <p style={mutedStyle}>
              Couldn&apos;t load your sharing state. Refresh to retry — if it
              keeps failing, please report it.
            </p>
          </div>
        </section>
      ) : (
        <>
          {/* Visibility — audit §05.7 calls for this to read as a top-
              level section header with the toggle inline, not as a buried
              card body row. Public state additionally surfaces the
              shareable URL inline so the user can copy it without
              hopping to the profile page. */}
          <section
            className="ss-card"
            aria-labelledby="ss-visibility-heading"
          >
            <header
              style={{
                ...cardHeaderStyle,
                display: 'flex',
                justifyContent: 'space-between',
                alignItems: 'center',
                gap: 16,
                flexWrap: 'wrap',
                paddingBottom: 14,
              }}
            >
              <div style={{ minWidth: 240, flex: 1 }}>
                <div className="ss-eyebrow" style={{ marginBottom: 6 }}>
                  Visibility
                </div>
                <h2 id="ss-visibility-heading" style={cardTitleStyle}>
                  {visibility?.public
                    ? 'Profile is public'
                    : 'Profile is private'}
                </h2>
              </div>
              <div
                style={{ display: 'flex', alignItems: 'center', gap: 10 }}
              >
                <span
                  className={`ss-badge ${visibility?.public ? 'ss-badge--ok' : ''}`}
                >
                  {visibility?.public ? (
                    <>
                      <span className="ss-badge-dot" />
                      Public
                    </>
                  ) : (
                    'Private'
                  )}
                </span>
                <form action={visibilityAction} style={{ margin: 0 }}>
                  <input
                    type="hidden"
                    name="public"
                    value={visibility?.public ? 'false' : 'true'}
                  />
                  <button
                    type="submit"
                    className={`ss-btn ${visibility?.public ? 'ss-btn--ghost' : 'ss-btn--primary'}`}
                  >
                    {visibility?.public ? 'Make private' : 'Make public'}
                  </button>
                </form>
              </div>
            </header>
            <div style={cardBodyStyle}>
              <p style={mutedStyle}>
                When public, anyone can view your summary and timeline at
                the URL below.
              </p>
              {visibility?.public ? (
                <div
                  style={{
                    display: 'flex',
                    gap: 8,
                    alignItems: 'center',
                    flexWrap: 'wrap',
                  }}
                >
                  <input
                    readOnly
                    value={`/u/${session.claimedHandle}`}
                    aria-label="Shareable public URL"
                    className="mono"
                    style={{
                      flex: '1 1 280px',
                      padding: '8px 12px',
                      background: 'var(--bg-elev)',
                      border: '1px solid var(--border)',
                      borderRadius: 'var(--r-sm)',
                      color: 'var(--fg)',
                      fontSize: 13,
                    }}
                  />
                  <Link
                    href={
                      (`/u/${encodeURIComponent(session.claimedHandle)}`) as Route
                    }
                    className="ss-btn ss-btn--ghost"
                    style={{ textDecoration: 'none' }}
                  >
                    Open
                  </Link>
                </div>
              ) : null}
              {/* Piece 4 — `/discover` listing sub-toggle. Default
                  is "show me in the listings" (listing_opt_out =
                  false). Disabled when the profile is private,
                  because a private profile already cannot appear on
                  /discover — the SpiceDB membership pre-filter would
                  drop it. We render the disabled state explicitly so
                  the user understands the dependency rather than
                  seeing the control disappear when they go private. */}
              {(() => {
                const isPublic = visibility?.public === true;
                const isOptedOut = visibility?.listing_opt_out === true;
                return (
                  <form
                    action={listingOptOutAction}
                    style={{
                      margin: 0,
                      paddingTop: 6,
                      borderTop: '1px dashed var(--border)',
                      display: 'flex',
                      alignItems: 'center',
                      justifyContent: 'space-between',
                      gap: 12,
                      flexWrap: 'wrap',
                    }}
                    data-testid="listing-opt-out-form"
                  >
                    <div style={{ minWidth: 240, flex: 1 }}>
                      <div
                        style={{
                          fontSize: 13,
                          fontWeight: 600,
                          color: isPublic ? 'var(--fg)' : 'var(--fg-muted)',
                        }}
                      >
                        Show me in the public profile listings at /discover
                      </div>
                      <p
                        style={{
                          ...mutedStyle,
                          marginTop: 4,
                        }}
                      >
                        {isPublic
                          ? isOptedOut
                            ? 'Hidden from the browsable index. Direct-URL access still works.'
                            : 'Listed alongside other public profiles. Direct-URL access unchanged.'
                          : 'Available only when your profile is public. Go public to control listing visibility.'}
                      </p>
                    </div>
                    <input
                      type="hidden"
                      name="listing_opt_out"
                      value={isOptedOut ? 'false' : 'true'}
                    />
                    <button
                      type="submit"
                      className="ss-btn ss-btn--ghost"
                      disabled={!isPublic}
                      data-testid="listing-opt-out-toggle"
                    >
                      {isOptedOut ? 'Show me' : 'Hide me'}
                    </button>
                  </form>
                );
              })()}
            </div>
          </section>

          {/* Profile views ---------------------------------------------- */}
          <ProfileViewsCard
            stats={profileViews}
            isPublic={visibility?.public === true}
          />

          {/* Outbound — handles ----------------------------------------- */}
          <section className="ss-card">
            <header style={cardHeaderStyle}>
              <div className="ss-eyebrow" style={{ marginBottom: 6 }}>
                Outbound · handles
              </div>
              <h2 style={cardTitleStyle}>Shared with specific handles</h2>
            </header>
            <div style={cardBodyStyle}>
              {(() => {
                // Audit v2.1 §A3 — bulk-ops row above the outbound list.
                // Render only the buttons whose preconditions are met;
                // an empty row is uglier than just hiding it.
                const now = Date.now();
                const expiredCount = (shares?.shares ?? []).filter(
                  (s) =>
                    s.expires_at && new Date(s.expires_at).getTime() <= now,
                ).length;
                const activeCount = (shares?.shares ?? []).filter(
                  (s) =>
                    !s.expires_at ||
                    new Date(s.expires_at).getTime() > now,
                ).length;
                if (expiredCount === 0 && activeCount === 0) return null;
                return (
                  <div
                    style={{
                      display: 'flex',
                      flexWrap: 'wrap',
                      gap: 8,
                      marginBottom: 12,
                      paddingBottom: 12,
                      borderBottom: '1px solid var(--border)',
                      alignItems: 'center',
                    }}
                  >
                    <span
                      style={{ fontSize: 11, color: 'var(--fg-dim)' }}
                    >
                      Bulk:
                    </span>
                    {expiredCount > 0 && (
                      <form
                        action={bulkRevokeExpiredAction}
                        style={{ margin: 0 }}
                      >
                        <button
                          type="submit"
                          className="ss-btn ss-btn--link"
                          style={{ color: 'var(--danger)' }}
                          title={`Revoke ${expiredCount} expired share${
                            expiredCount === 1 ? '' : 's'
                          }`}
                        >
                          Revoke {expiredCount} expired
                        </button>
                      </form>
                    )}
                    {activeCount > 0 && (
                      <form
                        action={bulkResetScopeAction}
                        style={{
                          margin: 0,
                          display: 'inline-flex',
                          alignItems: 'center',
                          gap: 6,
                        }}
                      >
                        <label
                          style={{
                            fontSize: 12,
                            color: 'var(--fg-muted)',
                            display: 'inline-flex',
                            alignItems: 'center',
                            gap: 4,
                          }}
                        >
                          Reset all to
                          <select
                            name="scope_kind"
                            defaultValue="aggregates"
                            style={{
                              fontSize: 12,
                              padding: '4px 6px',
                              background: 'var(--bg-elev)',
                              border: '1px solid var(--border)',
                              borderRadius: 'var(--r-sm)',
                              color: 'var(--fg)',
                            }}
                          >
                            <option value="full">full</option>
                            <option value="timeline">timeline</option>
                            <option value="aggregates">aggregates</option>
                          </select>
                        </label>
                        <button
                          type="submit"
                          className="ss-btn ss-btn--link"
                          title={`Reset scope on ${activeCount} active share${
                            activeCount === 1 ? '' : 's'
                          }`}
                        >
                          Apply
                        </button>
                      </form>
                    )}
                  </div>
                );
              })()}
              {shares && shares.shares.length > 0 ? (
                <div
                  style={{ display: 'flex', flexDirection: 'column', gap: 8 }}
                >
                  {shares.shares.map((entry) => {
                    const expiryLabel = formatExpiry(entry.expires_at);
                    // Audit v2 §05.2 — owner-visible activity hint
                    // beneath each pill. View_count + last_viewed_at
                    // come from the audit-log GROUP BY done server-
                    // side; we just render them.
                    const lastViewed = formatRelativePast(entry.last_viewed_at);
                    const viewCount = entry.view_count ?? 0;
                    const activityBits: string[] = [];
                    if (viewCount === 0) {
                      activityBits.push('not yet viewed');
                    } else {
                      activityBits.push(
                        `viewed ${viewCount} ${viewCount === 1 ? 'time' : 'times'}`,
                      );
                      if (lastViewed) activityBits.push(`last ${lastViewed}`);
                    }
                    if (entry.scope?.kind && entry.scope.kind !== 'full') {
                      activityBits.push(`scope: ${entry.scope.kind}`);
                    }
                    return (
                      <div
                        key={entry.recipient_handle}
                        style={{ ...sharePillStyle, flexWrap: 'wrap' }}
                      >
                        <div
                          style={{
                            display: 'flex',
                            flexDirection: 'column',
                            gap: 2,
                            flex: 1,
                            minWidth: 0,
                          }}
                        >
                          <Link
                            href={
                              (`/u/${encodeURIComponent(entry.recipient_handle)}`) as Route
                            }
                            className="mono"
                            style={{ color: 'var(--fg)' }}
                          >
                            {entry.recipient_handle}
                          </Link>
                          {entry.note && (
                            <span
                              style={{
                                fontSize: 12,
                                color: 'var(--fg-muted)',
                              }}
                            >
                              {entry.note}
                            </span>
                          )}
                          <span
                            style={{
                              fontSize: 11,
                              color: 'var(--fg-muted)',
                              letterSpacing: '0.01em',
                            }}
                            title={entry.last_viewed_at ?? undefined}
                          >
                            {activityBits.join(' · ')}
                          </span>
                        </div>
                        {expiryLabel && (
                          <span
                            className="ss-badge"
                            title={entry.expires_at ?? undefined}
                            style={
                              expiryLabel === 'expired'
                                ? { borderColor: 'var(--danger)', color: 'var(--danger)' }
                                : { color: 'var(--fg-muted)' }
                            }
                          >
                            {expiryLabel === 'expired' ? 'expired' : `expires ${expiryLabel}`}
                          </span>
                        )}
                        <Link
                          href={
                            buildEditHref(
                              entry.recipient_handle,
                              entry.expires_at,
                              entry.note,
                            ) as Route
                          }
                          className="ss-btn ss-btn--link"
                        >
                          Edit
                        </Link>
                        <form action={revokeShareAction} style={{ margin: 0 }}>
                          <input
                            type="hidden"
                            name="recipient_handle"
                            value={entry.recipient_handle}
                          />
                          <button
                            type="submit"
                            className="ss-btn ss-btn--link"
                            style={{ color: 'var(--danger)' }}
                          >
                            Revoke
                          </button>
                        </form>
                      </div>
                    );
                  })}
                </div>
              ) : (
                <p style={mutedStyle}>
                  You haven&apos;t shared with any specific handles yet.
                </p>
              )}
              <form
                id="share-editor"
                action={addShareAction}
                style={{
                  display: 'flex',
                  flexDirection: 'column',
                  gap: 8,
                }}
              >
                {isEditing && (
                  <p
                    style={{
                      ...mutedStyle,
                      fontSize: 12,
                      color: 'var(--accent)',
                    }}
                  >
                    Editing share with <span className="mono">{prefilledHandle}</span>
                    {' '}— blank out a field and save to clear it.
                  </p>
                )}
                <div style={formRowStyle}>
                  <input
                    type="text"
                    name="recipient_handle"
                    placeholder="RSI handle"
                    defaultValue={prefilledHandle}
                    autoComplete="off"
                    spellCheck={false}
                    className="mono"
                    required
                    maxLength={64}
                    readOnly={isEditing}
                    style={{
                      flex: '1 1 220px',
                      padding: '8px 12px',
                      background: 'var(--bg-elev)',
                      border: '1px solid var(--border)',
                      borderRadius: 'var(--r-sm)',
                      color: 'var(--fg)',
                      opacity: isEditing ? 0.7 : 1,
                    }}
                  />
                  <ExpiryField
                    prefillIso={prefilledExpires || undefined}
                    style={{
                      flex: '0 1 220px',
                      padding: '8px 12px',
                      background: 'var(--bg-elev)',
                      border: '1px solid var(--border)',
                      borderRadius: 'var(--r-sm)',
                      color: 'var(--fg)',
                    }}
                  />
                  <button type="submit" className="ss-btn ss-btn--primary">
                    {isEditing ? 'Save changes' : 'Grant access'}
                  </button>
                  {/* Audit v2.1 §B1 — preview opens a new tab with the
                      owner's own data run through the current scope. */}
                  <PreviewButton />
                  {isEditing && (
                    <Link
                      href={'/sharing' as Route}
                      className="ss-btn ss-btn--ghost"
                      style={{ textDecoration: 'none' }}
                    >
                      Cancel
                    </Link>
                  )}
                </div>
                <input
                  type="text"
                  name="note"
                  placeholder="Note (optional, max 280 chars)"
                  defaultValue={prefilledNote}
                  maxLength={280}
                  aria-label="Note (optional)"
                  style={{
                    padding: '8px 12px',
                    background: 'var(--bg-elev)',
                    border: '1px solid var(--border)',
                    borderRadius: 'var(--r-sm)',
                    color: 'var(--fg)',
                  }}
                />
                {/* Audit v2.1 §A2 — quick-start presets above the
                    detailed picker. Pure client-side; chips write
                    into the same scope_* fields the server action
                    reads. */}
                <ScopePresets />
                {/* Scope picker (audit §05.1) — hidden behind a
                    `<details>` so the existing two-line grant form
                    stays the default. Pure native HTML so the page
                    can remain a server component; the server action
                    above reads the values straight off FormData. */}
                <details style={{ marginTop: 4 }}>
                  <summary
                    style={{
                      cursor: 'pointer',
                      fontSize: 12,
                      color: 'var(--fg-muted)',
                      padding: '4px 0',
                    }}
                  >
                    Customise scope — default is full manifest
                  </summary>
                  <div
                    style={{
                      display: 'flex',
                      flexDirection: 'column',
                      gap: 10,
                      padding: '10px 0 4px',
                    }}
                  >
                    <label
                      style={{
                        display: 'flex',
                        flexDirection: 'column',
                        gap: 4,
                        fontSize: 12,
                        color: 'var(--fg-muted)',
                      }}
                    >
                      Scope kind
                      <select
                        name="scope_kind"
                        defaultValue="full"
                        style={{
                          padding: '8px 12px',
                          background: 'var(--bg-elev)',
                          border: '1px solid var(--border)',
                          borderRadius: 'var(--r-sm)',
                          color: 'var(--fg)',
                        }}
                      >
                        <option value="full">Full manifest (default)</option>
                        <option value="timeline">Timeline only</option>
                        <option value="aggregates">Aggregates only</option>
                        <option value="tabs">Specific tabs…</option>
                      </select>
                    </label>
                    <fieldset
                      style={{
                        border: '1px solid var(--border)',
                        borderRadius: 'var(--r-sm)',
                        padding: '8px 12px',
                        margin: 0,
                      }}
                    >
                      <legend
                        style={{
                          fontSize: 12,
                          color: 'var(--fg-muted)',
                          padding: '0 4px',
                        }}
                      >
                        Tabs (used when scope kind = tabs)
                      </legend>
                      <div
                        style={{
                          display: 'flex',
                          gap: 12,
                          flexWrap: 'wrap',
                        }}
                      >
                        {SCOPE_TAB_OPTIONS.map((t) => (
                          <label
                            key={t.value}
                            style={{
                              display: 'inline-flex',
                              alignItems: 'center',
                              gap: 6,
                              fontSize: 13,
                            }}
                          >
                            <input
                              type="checkbox"
                              name="scope_tabs"
                              value={t.value}
                            />
                            {t.label}
                          </label>
                        ))}
                      </div>
                    </fieldset>
                    <label
                      style={{
                        display: 'flex',
                        flexDirection: 'column',
                        gap: 4,
                        fontSize: 12,
                        color: 'var(--fg-muted)',
                      }}
                    >
                      Window (days, 1–90 — blank = no clamp)
                      <input
                        type="number"
                        name="scope_window_days"
                        min={1}
                        max={90}
                        placeholder="e.g. 7"
                        style={{
                          padding: '8px 12px',
                          background: 'var(--bg-elev)',
                          border: '1px solid var(--border)',
                          borderRadius: 'var(--r-sm)',
                          color: 'var(--fg)',
                        }}
                      />
                    </label>
                    <label
                      style={{
                        display: 'flex',
                        flexDirection: 'column',
                        gap: 4,
                        fontSize: 12,
                        color: 'var(--fg-muted)',
                      }}
                    >
                      Allow event types (comma-separated, blank = all)
                      <input
                        type="text"
                        name="scope_allow_event_types"
                        placeholder="quantum_target_selected, jump_completed"
                        className="mono"
                        style={{
                          padding: '8px 12px',
                          background: 'var(--bg-elev)',
                          border: '1px solid var(--border)',
                          borderRadius: 'var(--r-sm)',
                          color: 'var(--fg)',
                        }}
                      />
                    </label>
                    <label
                      style={{
                        display: 'flex',
                        flexDirection: 'column',
                        gap: 4,
                        fontSize: 12,
                        color: 'var(--fg-muted)',
                      }}
                    >
                      Deny event types (comma-separated)
                      <input
                        type="text"
                        name="scope_deny_event_types"
                        placeholder="actor_death"
                        className="mono"
                        style={{
                          padding: '8px 12px',
                          background: 'var(--bg-elev)',
                          border: '1px solid var(--border)',
                          borderRadius: 'var(--r-sm)',
                          color: 'var(--fg)',
                        }}
                      />
                    </label>
                  </div>
                </details>
              </form>
            </div>
          </section>

          {/* Outbound — orgs -------------------------------------------- */}
          <section className="ss-card">
            <header style={cardHeaderStyle}>
              <div className="ss-eyebrow" style={{ marginBottom: 6 }}>
                Outbound · orgs
              </div>
              <h2 style={cardTitleStyle}>Shared with orgs</h2>
            </header>
            <div style={cardBodyStyle}>
              {shares && (shares.org_shares?.length ?? 0) > 0 ? (
                <div
                  style={{ display: 'flex', flexDirection: 'column', gap: 8 }}
                >
                  {/* Resolve org name from the user's own org list so the
                      pill is readable ("Aurora Wing", not just
                      "aurora-wing"). When the user has shared with an
                      org they're no longer a member of, the lookup
                      misses and the slug stands in — clear enough that
                      the row is still actionable. */}
                  {(() => {
                    const orgNameBySlug = new Map(
                      (myOrgs?.orgs ?? []).map((o) => [o.slug, o.name]),
                    );
                    return (shares.org_shares ?? []).map((entry) => {
                      const name = orgNameBySlug.get(entry.org_slug);
                      return (
                        <div key={entry.org_slug} style={sharePillStyle}>
                          <Link
                            href={
                              (`/orgs/${encodeURIComponent(entry.org_slug)}`) as Route
                            }
                            style={{
                              display: 'flex',
                              flexDirection: 'column',
                              gap: 2,
                              color: 'var(--fg)',
                              textDecoration: 'none',
                              minWidth: 0,
                            }}
                          >
                            <span style={{ fontWeight: 500 }}>
                              {name ?? entry.org_slug}
                            </span>
                            {name && (
                              <span
                                className="mono"
                                style={{
                                  fontSize: 11,
                                  color: 'var(--fg-muted)',
                                }}
                              >
                                {entry.org_slug}
                              </span>
                            )}
                          </Link>
                          <form
                            action={revokeOrgShareAction}
                            style={{ margin: 0 }}
                          >
                            <input
                              type="hidden"
                              name="org_slug"
                              value={entry.org_slug}
                            />
                            <button
                              type="submit"
                              className="ss-btn ss-btn--link"
                              style={{ color: 'var(--danger)' }}
                            >
                              Revoke
                            </button>
                          </form>
                        </div>
                      );
                    });
                  })()}
                </div>
              ) : (
                <p style={mutedStyle}>
                  You haven&apos;t shared with any orgs yet.
                </p>
              )}
              {myOrgs && myOrgs.orgs.length > 0 ? (
                <form action={shareOrgAction} style={formRowStyle}>
                  <select
                    name="org_slug"
                    required
                    className="mono"
                    defaultValue=""
                    style={{
                      flex: '1 1 220px',
                      padding: '8px 12px',
                      background: 'var(--bg-elev)',
                      border: '1px solid var(--border)',
                      borderRadius: 'var(--r-sm)',
                      color: 'var(--fg)',
                    }}
                  >
                    <option value="" disabled>
                      Pick one of your orgs…
                    </option>
                    {myOrgs.orgs.map((o) => (
                      <option key={o.slug} value={o.slug}>
                        {o.name} ({o.slug})
                      </option>
                    ))}
                  </select>
                  <button type="submit" className="ss-btn ss-btn--primary">
                    Grant access
                  </button>
                </form>
              ) : (
                <p style={mutedStyle}>
                  You&apos;re not in any orgs yet —{' '}
                  <Link href="/orgs/new">create one</Link> to share by org.
                </p>
              )}
            </div>
          </section>

          {/* Inbound — what's been shared with me ------------------------ */}
          <section className="ss-card">
            <header style={cardHeaderStyle}>
              <div className="ss-eyebrow" style={{ marginBottom: 6 }}>
                Inbound · shared with you
              </div>
              <h2 style={cardTitleStyle}>People sharing with you</h2>
            </header>
            <div style={cardBodyStyle}>
              <p style={mutedStyle}>
                These owners have granted you view-access to their manifest.
                Org-mediated shares (via shared orgs) aren&apos;t listed here —
                check the org&apos;s detail page for those.
              </p>
              {(() => {
                const entries = inbound?.shared_with_me ?? [];
                const now = Date.now();
                const isExpired = (e: typeof entries[number]) =>
                  e.expires_at !== null &&
                  e.expires_at !== undefined &&
                  new Date(e.expires_at).getTime() <= now;
                const active = entries.filter((e) => !isExpired(e));
                const expired = entries.filter(isExpired);

                if (entries.length === 0) {
                  return (
                    <p style={mutedStyle}>
                      Nobody has shared their manifest with you yet.
                    </p>
                  );
                }

                return (
                  <>
                    {active.length > 0 && (
                      <div
                        style={{
                          display: 'flex',
                          flexDirection: 'column',
                          gap: 8,
                        }}
                      >
                        {active.map((entry) => (
                          <InboundPill
                            key={entry.owner_handle}
                            entry={entry}
                          />
                        ))}
                      </div>
                    )}
                    {expired.length > 0 && (
                      <details
                        style={{ marginTop: active.length > 0 ? 10 : 0 }}
                      >
                        <summary
                          style={{
                            cursor: 'pointer',
                            fontSize: 12,
                            color: 'var(--fg-muted)',
                            padding: '4px 0',
                          }}
                        >
                          {expired.length} expired{' '}
                          {expired.length === 1 ? 'share' : 'shares'} — owner
                          set an expiry that has now passed
                        </summary>
                        <div
                          style={{
                            display: 'flex',
                            flexDirection: 'column',
                            gap: 8,
                            marginTop: 8,
                            opacity: 0.55,
                          }}
                        >
                          {expired.map((entry) => (
                            <InboundPill
                              key={entry.owner_handle}
                              entry={entry}
                            />
                          ))}
                        </div>
                      </details>
                    )}
                    {active.length === 0 && expired.length > 0 && (
                      <p style={mutedStyle}>
                        Every share you have has expired. Nothing new right
                        now.
                      </p>
                    )}
                  </>
                );
              })()}
            </div>
          </section>
        </>
      )}
    </div>
  );
}

/**
 * Inbound share row. Extracted because both the active list and the
 * collapsed expired-shares group render the same pill — keeping the
 * markup in one place is the only way the two sub-lists stay visually
 * aligned as styling evolves.
 */
function InboundPill({
  entry,
}: {
  entry: SharedWithMeEntry;
}) {
  const expiryLabel = formatExpiry(entry.expires_at);
  return (
    <div
      style={{
        ...sharePillStyle,
        flexWrap: 'wrap',
        flexDirection: 'column',
        alignItems: 'stretch',
        gap: 8,
      }}
    >
      <div style={{ display: 'flex', gap: 8, flexWrap: 'wrap' }}>
        <div
          style={{
            display: 'flex',
            flexDirection: 'column',
            gap: 2,
            flex: 1,
            minWidth: 0,
          }}
        >
          <Link
            href={(`/u/${encodeURIComponent(entry.owner_handle)}`) as Route}
            className="mono"
            style={{ color: 'var(--fg)' }}
          >
            @{entry.owner_handle}
          </Link>
          {entry.note && (
            <span style={{ fontSize: 12, color: 'var(--fg-muted)' }}>
              {entry.note}
            </span>
          )}
        </div>
        {expiryLabel && (
          <span
            className="ss-badge"
            title={entry.expires_at ?? undefined}
            style={
              expiryLabel === 'expired'
                ? { borderColor: 'var(--danger)', color: 'var(--danger)' }
                : { color: 'var(--fg-muted)' }
            }
          >
            {expiryLabel === 'expired' ? 'expired' : `expires ${expiryLabel}`}
          </span>
        )}
        <Link
          href={(`/u/${encodeURIComponent(entry.owner_handle)}`) as Route}
          className="ss-btn ss-btn--link"
        >
          View profile
        </Link>
      </div>

      {/* Audit v2 §05 — recipient-facing report affordance. Collapsed
          by default so the row stays compact; opens an inline form
          that posts to /v1/share/report via the server action. */}
      <details>
        <summary
          style={{
            cursor: 'pointer',
            fontSize: 12,
            color: 'var(--fg-muted)',
          }}
        >
          Report this share
        </summary>
        <form
          action={reportShareAction}
          style={{
            display: 'flex',
            flexDirection: 'column',
            gap: 6,
            marginTop: 8,
            padding: 10,
            border: '1px solid var(--border)',
            borderRadius: 'var(--r-card)',
            background: 'var(--bg-sunken)',
          }}
        >
          <input
            type="hidden"
            name="owner_handle"
            value={entry.owner_handle}
          />
          {/* recipient_handle (the reporter) is NOT sent — the action
              reads it from session.claimedHandle (server-side handle
              truth). Only the OTHER party (owner_handle) comes from the
              client. (M-W2) */}
          <label
            style={{
              display: 'flex',
              flexDirection: 'column',
              gap: 4,
              fontSize: 12,
            }}
          >
            <span style={{ color: 'var(--fg-muted)' }}>Reason</span>
            <select
              name="reason"
              required
              defaultValue="abuse"
              style={{
                padding: 6,
                background: 'var(--bg-elev)',
                color: 'var(--fg)',
                border: '1px solid var(--border)',
                borderRadius: 'var(--r-input)',
                fontSize: 13,
              }}
            >
              <option value="abuse">Abuse</option>
              <option value="spam">Spam</option>
              <option value="data_misuse">Data misuse</option>
              <option value="other">Other</option>
            </select>
          </label>
          <label
            style={{
              display: 'flex',
              flexDirection: 'column',
              gap: 4,
              fontSize: 12,
            }}
          >
            <span style={{ color: 'var(--fg-muted)' }}>
              Details (optional, ≤ 500 chars)
            </span>
            <textarea
              name="details"
              rows={2}
              maxLength={500}
              style={{
                resize: 'vertical',
                fontFamily: 'inherit',
                fontSize: 13,
                padding: 6,
                background: 'var(--bg-elev)',
                color: 'var(--fg)',
                border: '1px solid var(--border)',
                borderRadius: 'var(--r-input)',
              }}
            />
          </label>
          <button type="submit" className="ss-btn">
            Submit report
          </button>
        </form>
      </details>
    </div>
  );
}

/**
 * Profile views card. Renders directly under the visibility toggle on
 * /sharing. When the profile is private, shows a placeholder pointing
 * the user at the toggle above. When public, surfaces the 30-day
 * total, a per-source breakdown line, and a sparkline of per-day
 * counts.
 *
 * Bars are plain CSS divs (no chart library) — the data is tiny
 * (<=30 entries) and the bar widths scale off the per-day max so a
 * single big day doesn't make every other day look empty.
 */
function ProfileViewsCard({
  stats,
  isPublic,
}: {
  stats: ProfileViewStats | null;
  isPublic: boolean;
}) {
  if (!isPublic) {
    return (
      <section className="ss-card" data-testid="profile-views-card">
        <header style={cardHeaderStyle}>
          <div className="ss-eyebrow" style={{ marginBottom: 6 }}>
            Profile views
          </div>
          <h2 style={cardTitleStyle}>Tracking is off</h2>
        </header>
        <div style={cardBodyStyle}>
          <p style={mutedStyle}>
            Make your profile public to start tracking views.
          </p>
        </div>
      </section>
    );
  }
  // Public but the fetch failed or no rows yet — keep the card so the
  // user knows the feature exists.
  const totals = stats?.totals;
  const last30 = totals?.last_30d ?? 0;
  const days = stats?.days ?? [];
  // Reverse to oldest-first so the sparkline reads left -> right.
  const sparkline = [...days].reverse();
  const maxDay = sparkline.reduce((acc, d) => Math.max(acc, d.total), 0);

  return (
    <section className="ss-card" data-testid="profile-views-card">
      <header style={cardHeaderStyle}>
        <div className="ss-eyebrow" style={{ marginBottom: 6 }}>
          Profile views
        </div>
        <h2 style={cardTitleStyle}>Last 30 days</h2>
      </header>
      <div style={cardBodyStyle}>
        {last30 === 0 ? (
          <p style={mutedStyle}>No views yet.</p>
        ) : (
          <>
            <div
              data-testid="profile-views-total"
              style={{
                fontSize: 36,
                fontWeight: 600,
                letterSpacing: '-0.02em',
                color: 'var(--fg)',
                lineHeight: 1,
              }}
            >
              {last30}
            </div>
            <p
              data-testid="profile-views-breakdown"
              style={{ ...mutedStyle, marginTop: 4 }}
            >
              {renderBreakdown(totals?.by_source_30d ?? {})}
            </p>
            {sparkline.length > 0 && maxDay > 0 && (
              <div
                aria-label="Daily view counts for the last 30 days"
                data-testid="profile-views-sparkline"
                style={{
                  display: 'flex',
                  alignItems: 'flex-end',
                  gap: 2,
                  height: 48,
                  marginTop: 6,
                  paddingTop: 4,
                  borderTop: '1px solid var(--border)',
                }}
              >
                {sparkline.map((d) => {
                  // Floor at 6% so a non-zero day is still visible — a
                  // 1-view-against-a-100-view-day would otherwise be
                  // 1px and indistinguishable from an empty bucket.
                  const pct =
                    d.total === 0
                      ? 0
                      : Math.max(6, Math.round((d.total / maxDay) * 100));
                  return (
                    <div
                      key={d.day}
                      title={`${d.day}: ${d.total} ${
                        d.total === 1 ? 'view' : 'views'
                      }`}
                      style={{
                        flex: 1,
                        height: `${pct}%`,
                        minWidth: 2,
                        background:
                          d.total === 0
                            ? 'var(--border)'
                            : 'var(--accent, var(--fg))',
                        borderRadius: 2,
                        opacity: d.total === 0 ? 0.35 : 1,
                      }}
                    />
                  );
                })}
              </div>
            )}
          </>
        )}
      </div>
    </section>
  );
}

/** Pretty-print the by_source_30d map as
 *  "12 from direct links · 8 from discover · 3 from shared profiles".
 *  Hidden buckets (zero counts, unknown sources) are skipped so the
 *  line stays readable. */
function renderBreakdown(bySource: Record<string, number>): string {
  const labels: Array<[string, string]> = [
    ['direct', 'from direct links'],
    ['discover', 'from discover'],
    ['shared', 'from shared profiles'],
    ['other', 'from other'],
  ];
  const parts: string[] = [];
  for (const [key, label] of labels) {
    const n = bySource[key] ?? 0;
    if (n > 0) parts.push(`${n} ${label}`);
  }
  return parts.length === 0 ? 'No source data yet.' : parts.join(' · ');
}
