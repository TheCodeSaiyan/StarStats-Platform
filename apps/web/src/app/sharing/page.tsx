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
  type VisibilityResponse,
} from '@/lib/api';
import { localInputToUtcIso } from '@/lib/expiry';
import { logger } from '@/lib/logger';
import { Plane, BeamAlert, BeamButton, BeamChip, BeamInput, BeamSelect, SubStats } from 'holo';
import { navSections } from '@/lib/nav';
import { setCalibrationAction } from '@/app/me/_projection/actions';
import { getTheme } from '@/lib/theme';
import type { Calibration } from 'holo';
import {
  SharingProjection,
  type SharingSection,
} from './_projection/SharingProjection';
import { ShareEditor } from './_projection/ShareEditor';
import { InboundList } from './_projection/InboundList';
import { ProfileViewsPane } from './_projection/ProfileViewsPane';
import { formatExpiry, formatRelativePast } from './_projection/format';
import { getSession } from '@/lib/session';
import {
  bulkResetScopeAction,
  bulkRevokeExpiredAction,
  reportShareAction,
} from './actions';

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


export default async function SharingPage(props: {
  searchParams: Promise<SearchParams>;
}) {
  const session = await getSession();
  if (!session) redirect('/auth/login?next=/sharing');

  const params = await props.searchParams;
  const status = params.status;
  // The beam, for this render. Falls back to the system default rather than
  // failing the page — a calibration is not worth a 500.
  let calibration: Calibration = 'terra';
  try {
    calibration = (await getTheme(session.token)) as Calibration;
  } catch (e) {
    logger.warn({ err: e, call: 'sharing.theme' }, 'load theme failed');
  }
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

  // ---------------------------------------------------------------------
  // Sections.
  //
  // The five real sections of the route. Nothing here comes from the kit's
  // `Sharing.jsx`, which COVERAGE marks inferred and calls "a sketch".
  //
  // Ids are LOAD-BEARING: `#share-editor` is how the edit flow works
  // (`?edit=<handle>#share-editor`), so it is declared as a secondary anchor
  // on the outbound section — otherwise the rail would not open the group
  // that contains the editor being scrolled to.
  // ---------------------------------------------------------------------
  const isPublic = visibility?.public === true;
  const isOptedOut = visibility?.listing_opt_out === true;
  const now = Date.now();
  const shareEntries = shares?.shares ?? [];
  const expiredCount = shareEntries.filter(
    (s) => s.expires_at && new Date(s.expires_at).getTime() <= now,
  ).length;
  const activeCount = shareEntries.filter(
    (s) => !s.expires_at || new Date(s.expires_at).getTime() > now,
  ).length;

  /*
   * `Sharing.jsx` leads with a SubStats row and one line of copy before any
   * control — Public / Org-only / Named grants / Inbound — so a reader learns
   * what they are currently exposing before they are asked to change it. The
   * port went straight to the controls, which reads as a settings form rather
   * than an answer to "who can see my data".
   *
   * Every figure is real: the profile flag, the org-share rows, the outbound
   * grants and the inbound list are all already on this page.
   */
  const orgShareCount = (shares?.org_shares ?? []).length;
  const inboundCount = (inbound?.shared_with_me ?? []).length;

  const sections: SharingSection[] = [
    {
      id: 'summary',
      title: 'Sharing',
      ctx: 'Per-element visibility',
      group: 'visibility',
      node: (
        <>
          <SubStats
            items={[
              {
                k: 'Profile',
                v: isPublic ? 'Public' : 'Private',
                tone: isPublic ? 'good' : undefined,
              },
              { k: 'Named grants', v: String(shareEntries.length) },
              { k: 'Org-only', v: String(orgShareCount) },
              {
                k: 'Inbound',
                v: String(inboundCount),
                tone: inboundCount > 0 ? 'good' : undefined,
              },
            ]}
          />
          {/* Shipped wording from the spec, which states the default plainly.
              A sharing page that does not say what happens to everything NOT
              listed is the one thing a reader actually needs to know. */}
          <p className="hp-prose">
            Visibility is per element, on top of the profile-level setting.
            Anything not listed here stays private.
          </p>
        </>
      ),
    },
    {
      id: 'visibility',
      title: isPublic ? 'Profile is public' : 'Profile is private',
      ctx: isPublic ? 'Anyone with the URL can read it' : 'Only you',
      group: 'visibility',
      node: (
        <>
          <div className="hp-statusline">
            <BeamChip tone={isPublic ? 'good' : undefined} dot={isPublic}>
              {isPublic ? 'Public' : 'Private'}
            </BeamChip>
            <span>
              When public, anyone can view your summary and timeline at the URL
              below.
            </span>
          </div>
          <form action={visibilityAction} className="hp-formcol">
            <input
              type="hidden"
              name="public"
              value={isPublic ? 'false' : 'true'}
            />
            <BeamButton
              type="submit"
              variant={isPublic ? 'ghost' : 'primary'}
              style={{ alignSelf: 'flex-start' }}
            >
              {isPublic ? 'Make private' : 'Make public'}
            </BeamButton>
          </form>

          {isPublic ? (
            <Plane tilt="flat" cap="Public URL" style={{ marginTop: 20 }}>
              <div className="hp-formrow" style={{ marginTop: 4 }}>
                <BeamInput
                  id="public-url"
                  readOnly
                  value={`/u/${session.claimedHandle}`}
                  aria-label="Shareable public URL"
                />
                <Link
                  href={
                    `/u/${encodeURIComponent(session.claimedHandle)}` as Route
                  }
                  className="hp-btn hp-btn--ghost"
                >
                  Open →
                </Link>
              </div>
            </Plane>
          ) : null}

          {/* The `/discover` listing sub-toggle. Rendered DISABLED rather than
              hidden when the profile is private: a private profile already
              cannot appear in the listings (the SpiceDB membership pre-filter
              drops it), and showing the dependency explains it where a
              disappearing control would just confuse. */}
          <Plane
            tilt="flat"
            cap="Directory listing"
            style={{ marginTop: 20 }}
          >
            <form
              action={listingOptOutAction}
              data-testid="listing-opt-out-form"
              className="hp-formcol"
            >
              <p className="hp-prose" style={{ marginTop: 0 }}>
                Show me in the public profile listings at /discover.
              </p>
              <p className="hp-prose" style={{ marginTop: 0 }}>
                {isPublic
                  ? isOptedOut
                    ? 'Hidden from the browsable index. Direct-URL access still works.'
                    : 'Listed alongside other public profiles. Direct-URL access unchanged.'
                  : 'Available only when your profile is public. Go public to control listing visibility.'}
              </p>
              <input
                type="hidden"
                name="listing_opt_out"
                value={isOptedOut ? 'false' : 'true'}
              />
              <BeamButton
                type="submit"
                variant="ghost"
                disabled={!isPublic}
                data-testid="listing-opt-out-toggle"
                style={{ alignSelf: 'flex-start' }}
              >
                {isOptedOut ? 'Show me' : 'Hide me'}
              </BeamButton>
            </form>
          </Plane>
        </>
      ),
    },

    {
      id: 'handles',
      title: 'Shared with specific handles',
      ctx: `${activeCount} active`,
      group: 'outbound',
      // `#share-editor` lives inside this section — see SurfaceSection.anchors.
      anchors: ['share-editor'],
      node: (
        <>
          {/* Bulk row. Only the buttons whose preconditions are met — an empty
              row is uglier than none. */}
          {expiredCount > 0 || activeCount > 0 ? (
            <div className="hp-bulkrow">
              <span className="hp-fieldlabel" style={{ margin: 0 }}>
                Bulk
              </span>
              {expiredCount > 0 ? (
                <form action={bulkRevokeExpiredAction}>
                  <BeamButton
                    type="submit"
                    variant="ghost"
                    title={`Revoke ${expiredCount} expired share${expiredCount === 1 ? '' : 's'}`}
                  >
                    Revoke {expiredCount} expired
                  </BeamButton>
                </form>
              ) : null}
              {activeCount > 0 ? (
                <form action={bulkResetScopeAction} className="hp-formrow" style={{ marginTop: 0 }}>
                  <BeamSelect
                    id="bulk-scope-kind"
                    name="scope_kind"
                    label="Reset all to"
                    defaultValue="aggregates"
                  >
                    <option value="full">full</option>
                    <option value="timeline">timeline</option>
                    <option value="aggregates">aggregates</option>
                  </BeamSelect>
                  <BeamButton
                    type="submit"
                    variant="ghost"
                    title={`Reset scope on ${activeCount} active share${activeCount === 1 ? '' : 's'}`}
                  >
                    Apply
                  </BeamButton>
                </form>
              ) : null}
            </div>
          ) : null}

          {shareEntries.length > 0 ? (
            <Plane tilt="flat" cap="Grants" hint={`${shareEntries.length}`} style={{ marginTop: 18 }}>
              {shareEntries.map((entry) => {
                const expiryLabel = formatExpiry(entry.expires_at);
                // Owner-visible activity hint. `view_count` and
                // `last_viewed_at` come from an audit-log GROUP BY done
                // server-side; this only renders them.
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
                  <div className="hp-grant" key={entry.recipient_handle}>
                    <div className="hp-grant__who">
                      <Link
                        href={
                          `/u/${encodeURIComponent(entry.recipient_handle)}` as Route
                        }
                      >
                        {entry.recipient_handle}
                      </Link>
                      {entry.note ? (
                        <span className="hp-grant__note">{entry.note}</span>
                      ) : null}
                      <span
                        className="hp-grant__act"
                        title={entry.last_viewed_at ?? undefined}
                      >
                        {activityBits.join(' · ')}
                      </span>
                    </div>
                    {expiryLabel ? (
                      <BeamChip
                        tone={expiryLabel === 'expired' ? 'bad' : undefined}
                        title={entry.expires_at ?? undefined}
                      >
                        {expiryLabel === 'expired'
                          ? 'expired'
                          : `expires ${expiryLabel}`}
                      </BeamChip>
                    ) : null}
                    <div className="hp-grant__act-btns">
                      {/* `buildEditHref`, not a hand-built URL: it round-trips
                          the share's CURRENT expiry and note through the query
                          so the editor pre-fills them, and it emits the expiry
                          as a UTC instant that `ExpiryField` localises back to
                          the wall-clock the owner originally picked. Rebuilding
                          the link by hand silently drops both. */}
                      <Link
                        href={
                          buildEditHref(
                            entry.recipient_handle,
                            entry.expires_at,
                            entry.note,
                          ) as Route
                        }
                      >
                        Edit
                      </Link>
                      <form action={revokeShareAction} style={{ margin: 0 }}>
                        <input
                          type="hidden"
                          name="recipient_handle"
                          value={entry.recipient_handle}
                        />
                        <BeamButton type="submit" variant="ghost">
                          Revoke
                        </BeamButton>
                      </form>
                    </div>
                  </div>
                );
              })}
            </Plane>
          ) : (
            <p className="hp-prose">
              You haven&apos;t shared your manifest with anyone yet.
            </p>
          )}

          <ShareEditor
            addShareAction={addShareAction}
            isEditing={isEditing}
            prefilledHandle={prefilledHandle}
            prefilledNote={prefilledNote}
            prefilledExpires={prefilledExpires}
          />
        </>
      ),
    },

    {
      id: 'orgs',
      title: 'Shared with orgs',
      group: 'outbound',
      node: (
        <>
          <p className="hp-prose">
            Everyone in the org can read what you share with it. Membership is
            RSI&apos;s, not ours — leaving the org removes the access.
          </p>
          {(shares?.org_shares ?? []).length > 0 ? (
            <Plane tilt="flat" cap="Org grants" style={{ marginTop: 18 }}>
              {(() => {
                const orgNames = new Map(
                  (myOrgs?.orgs ?? []).map((o) => [o.slug, o.name] as const),
                );
                return (shares?.org_shares ?? []).map((entry) => (
                  <div className="hp-grant" key={entry.org_slug}>
                    <div className="hp-grant__who">
                      <span>{orgNames.get(entry.org_slug) ?? entry.org_slug}</span>
                      <span className="hp-grant__act">{entry.org_slug}</span>
                    </div>
                    <div className="hp-grant__act-btns">
                      <form action={revokeOrgShareAction} style={{ margin: 0 }}>
                        <input
                          type="hidden"
                          name="org_slug"
                          value={entry.org_slug}
                        />
                        <BeamButton type="submit" variant="ghost">
                          Revoke
                        </BeamButton>
                      </form>
                    </div>
                  </div>
                ));
              })()}
            </Plane>
          ) : (
            <p className="hp-prose">No org shares yet.</p>
          )}

          {myOrgs && myOrgs.orgs.length > 0 ? (
            <form action={shareOrgAction} className="hp-formrow">
              <BeamSelect id="org-slug" name="org_slug" label="Org">
                {myOrgs.orgs.map((o) => (
                  <option key={o.slug} value={o.slug}>
                    {o.name}
                  </option>
                ))}
              </BeamSelect>
              <BeamButton type="submit" variant="primary">
                Share with org
              </BeamButton>
            </form>
          ) : (
            <p className="hp-prose">
              You aren&apos;t in any orgs we know about. Refresh your org
              snapshot from Calibrate → RSI handle.
            </p>
          )}
        </>
      ),
    },

    {
      id: 'inbound',
      title: 'People sharing with you',
      group: 'inbound',
      node: (
        <>
          <p className="hp-prose">
            These owners have granted you view-access to their manifest.
            Org-mediated shares aren&apos;t listed here — check the org&apos;s
            detail page for those.
          </p>
          <InboundList
            entries={inbound?.shared_with_me ?? []}
            reportShareAction={reportShareAction}
          />
        </>
      ),
    },

    {
      id: 'views',
      title: isPublic ? 'Profile views' : 'Tracking is off',
      ctx: isPublic ? 'Last 30 days' : undefined,
      group: 'views',
      node: <ProfileViewsPane stats={profileViews} isPublic={isPublic} />,
    },
  ];

  // `?status=` is a success and `?error=` a failure. Both map through the
  // tables the flat page used, so the copy is unchanged.
  const statusEntry = status ? STATUS_MESSAGES[status] : undefined;
  const notice = statusEntry
    ? {
        // `text` is sometimes a function of a count (a bulk action reports how
        // many it touched), so `?n=` is threaded through exactly as before.
        tone: (statusEntry.tone === 'ok' ? 'good' : 'bad') as 'good' | 'bad',
        message:
          typeof statusEntry.text === 'function'
            ? statusEntry.text(Number.parseInt(params.n ?? '0', 10) || 0)
            : statusEntry.text,
      }
    : errorCode
      ? {
          tone: 'bad' as const,
          message: ERROR_MESSAGES[errorCode] ?? errorCode,
        }
      : null;

  // Degraded service is a BANNER, not a section: when SpiceDB is down there is
  // nothing truthful to render in any group, so the page says so once at the
  // top rather than showing empty grants that read as "you have no shares".
  const banner =
    degraded === 'spicedb_unavailable' ? (
      <BeamAlert tone="bad">
        Sharing is temporarily unavailable — the authorisation service is
        offline. Try again shortly.
      </BeamAlert>
    ) : degraded === 'unknown' ? (
      <BeamAlert tone="bad">
        Couldn&apos;t load your sharing state. Refresh to retry — if it keeps
        failing, please report it.
      </BeamAlert>
    ) : null;

  return (
    <SharingProjection
      handle={session.claimedHandle}
      calibration={calibration}
      nav={navSections(
        { signedIn: true, staffRoles: session.staffRoles },
        'sharing',
      )}
      sections={degraded ? [] : sections}
      notice={notice}
      banner={banner}
      onCalibrate={async (id: string) => {
        'use server';
        await setCalibrationAction(id);
      }}
    />
  );
}



