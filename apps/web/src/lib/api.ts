/**
 * Thin wrapper around the StarStats API. Server-side only —
 * everything here runs in Server Components / Server Actions /
 * route handlers and the JWT never reaches the browser-side JS.
 *
 * The API URL is configured via STARSTATS_API_URL (server env).
 * In dev that's typically `http://localhost:8080`; in prod it
 * lives behind Traefik at `https://api.example.com`.
 *
 * Type contract: every response/request shape is a type alias over
 * the generated OpenAPI schema, imported as a workspace dep from the
 * `api-client-ts` package (sourced from
 * `packages/api-client-ts/src/generated/schema.ts`). The exported
 * type names here are kept stable so existing call sites don't churn.
 * To regenerate after server changes:
 *   pnpm --filter api-client-ts run generate
 */

import 'server-only';
import type { components as apiSchema } from 'api-client-ts';
import { IN_TRANSIT_HIDDEN_TYPES, filterMovementNoise } from './event-filter';

// Every response/request shape below is sourced from the generated
// OpenAPI schema (`packages/api-client-ts/src/generated/schema.ts`)
// rather than hand-rolled. The local `export type` names are kept
// stable so existing call sites don't churn — they're just aliases
// pointing at the codegen output. To regenerate after server changes:
//   pnpm --filter api-client-ts run generate
export type SummaryResponse = apiSchema['schemas']['SummaryResponse'];

export type AuthResponse = apiSchema['schemas']['AuthResponse'];

export type MeResponse = apiSchema['schemas']['MeResponse'];

export type ChangePasswordRequest =
  apiSchema['schemas']['ChangePasswordRequest'];
export type ChangePasswordResponse =
  apiSchema['schemas']['ChangePasswordResponse'];

export type DeleteAccountRequest =
  apiSchema['schemas']['DeleteAccountRequest'];
export type DeleteAccountResponse =
  apiSchema['schemas']['DeleteAccountResponse'];

export type ResendVerificationResponse =
  apiSchema['schemas']['ResendVerificationResponse'];

export type PasswordResetStartRequest =
  apiSchema['schemas']['PasswordResetStartRequest'];
export type PasswordResetStartResponse =
  apiSchema['schemas']['PasswordResetStartResponse'];
export type PasswordResetCompleteRequest =
  apiSchema['schemas']['PasswordResetCompleteRequest'];
export type PasswordResetCompleteResponse =
  apiSchema['schemas']['PasswordResetCompleteResponse'];

export type EmailChangeStartRequest =
  apiSchema['schemas']['EmailChangeStartRequest'];
export type EmailChangeStartResponse =
  apiSchema['schemas']['EmailChangeStartResponse'];
export type EmailChangeVerifyRequest =
  apiSchema['schemas']['EmailChangeVerifyRequest'];
export type EmailChangeVerifyResponse =
  apiSchema['schemas']['EmailChangeVerifyResponse'];

export type RsiStartResponse = apiSchema['schemas']['RsiStartResponse'];
export type RsiVerifyResponse = apiSchema['schemas']['RsiVerifyResponse'];

export type MagicLinkStartRequest =
  apiSchema['schemas']['MagicLinkStartRequest'];
export type MagicLinkStartResponse =
  apiSchema['schemas']['MagicLinkStartResponse'];
export type MagicLinkRedeemRequest =
  apiSchema['schemas']['MagicLinkRedeemRequest'];

export type TotpSetupResponse = apiSchema['schemas']['TotpSetupResponse'];
export type TotpConfirmRequest = apiSchema['schemas']['TotpConfirmRequest'];
export type TotpConfirmResponse = apiSchema['schemas']['TotpConfirmResponse'];
export type TotpDisableRequest = apiSchema['schemas']['TotpDisableRequest'];
export type TotpDisableResponse =
  apiSchema['schemas']['TotpDisableResponse'];
export type RegenerateRecoveryRequest =
  apiSchema['schemas']['RegenerateRecoveryRequest'];
export type RegenerateRecoveryResponse =
  apiSchema['schemas']['RegenerateRecoveryResponse'];
export type VerifyLoginRequest =
  apiSchema['schemas']['VerifyLoginRequest'];

export type TimelineBucket = apiSchema['schemas']['TimelineBucket'];
export type TimelineResponse = apiSchema['schemas']['TimelineResponse'];

// `PairingResponse` is the local name; the generated schema calls the
// same shape `StartResponse` (it's the body of POST /v1/auth/devices/start).
// The alias preserves the existing import name in callers.
export type PairingResponse = apiSchema['schemas']['StartResponse'];

export type DeviceListResponse = apiSchema['schemas']['DeviceListResponse'];

export type DeviceDto = apiSchema['schemas']['DeviceDto'];

export type SetSyncRequest = apiSchema['schemas']['SetSyncRequest'];
export type SetSyncResponse = apiSchema['schemas']['SetSyncResponse'];

export type VerifyEmailResponse = apiSchema['schemas']['VerifyEmailResponse'];

export type SmtpConfigResponse = apiSchema['schemas']['SmtpConfigResponse'];
export type SmtpConfigRequest = apiSchema['schemas']['SmtpConfigRequest'];
export type TestSendResponse = apiSchema['schemas']['TestSendResponse'];

export interface ApiError {
  error: string;
  detail?: string;
}

export class ApiCallError extends Error {
  constructor(
    public readonly status: number,
    public readonly body: ApiError,
  ) {
    super(`${status} ${body.error}${body.detail ? ` — ${body.detail}` : ''}`);
    this.name = 'ApiCallError';
  }
}

/**
 * HTTP status off a caught value when it's an `ApiCallError`, else
 * `undefined`. For structured `logger.warn({ call, status }, …)` on
 * `Promise.allSettled` rejections so the failing endpoint is named
 * with its code in server logs (the docs/ENGINEERING.md allSettled invariant).
 */
export function statusOf(e: unknown): number | undefined {
  return e instanceof ApiCallError ? e.status : undefined;
}

export function apiBase(): string {
  const raw = process.env.STARSTATS_API_URL;
  if (!raw) {
    throw new Error(
      'STARSTATS_API_URL is not set — point it at the Rust API origin',
    );
  }
  return raw.replace(/\/+$/, '');
}

// Hard ceiling on a single API call so a hung upstream (dead API,
// stalled network) surfaces as a thrown TimeoutError rather than an
// indefinitely-pending server render or action (L3). Generous enough
// for writes; the cosmetic reference fetchers use a tighter 8s budget.
const REQUEST_TIMEOUT_MS = 15_000;

async function request<T>(
  method: 'GET' | 'POST' | 'PUT' | 'PATCH' | 'DELETE',
  path: string,
  body: unknown | undefined,
  bearer: string | undefined,
): Promise<T> {
  const headers: Record<string, string> = {};
  if (body !== undefined) headers['content-type'] = 'application/json';
  if (bearer) headers.authorization = `Bearer ${bearer}`;

  const resp = await fetch(`${apiBase()}${path}`, {
    method,
    headers,
    body: body !== undefined ? JSON.stringify(body) : undefined,
    cache: 'no-store',
    signal: AbortSignal.timeout(REQUEST_TIMEOUT_MS),
  });

  if (resp.status === 204) {
    return undefined as T;
  }

  if (!resp.ok) {
    let errBody: ApiError;
    try {
      errBody = (await resp.json()) as ApiError;
    } catch {
      errBody = { error: `http_${resp.status}` };
    }
    throw new ApiCallError(resp.status, errBody);
  }

  return (await resp.json()) as T;
}

async function postJson<T>(
  path: string,
  body: unknown,
  bearer?: string,
): Promise<T> {
  return request<T>('POST', path, body, bearer);
}

async function putJson<T>(
  path: string,
  body: unknown,
  bearer?: string,
): Promise<T> {
  return request<T>('PUT', path, body, bearer);
}

export async function signup(input: {
  email: string;
  password: string;
  claimed_handle: string;
  /** Beta invite, minted by the waitlist on admission. The server
   *  REQUIRES it while `waitlist_config.gate_enabled` is true and
   *  ignores it otherwise, so it is always safe to send and must be
   *  sendable — until this existed, flipping the gate made signup
   *  impossible for everyone, invite holders included. */
  invite_token?: string;
}): Promise<AuthResponse> {
  return postJson<AuthResponse>('/v1/auth/signup', input);
}

export async function login(input: {
  email: string;
  password: string;
}): Promise<AuthResponse> {
  return postJson<AuthResponse>('/v1/auth/login', input);
}

export async function verifyEmail(input: {
  token: string;
}): Promise<VerifyEmailResponse> {
  return postJson<VerifyEmailResponse>('/v1/auth/email/verify', input);
}

export async function startPairing(
  bearer: string,
  input: { label?: string },
): Promise<PairingResponse> {
  return postJson<PairingResponse>('/v1/auth/devices/start', input, bearer);
}

export async function listDevices(bearer: string): Promise<DeviceListResponse> {
  return request<DeviceListResponse>(
    'GET',
    '/v1/auth/devices',
    undefined,
    bearer,
  );
}

export async function revokeDevice(
  bearer: string,
  deviceId: string,
): Promise<void> {
  await request<void>(
    'DELETE',
    `/v1/auth/devices/${encodeURIComponent(deviceId)}`,
    undefined,
    bearer,
  );
}

export async function setDeviceSync(
  bearer: string,
  deviceId: string,
  enabled: boolean,
): Promise<SetSyncResponse> {
  return postJson<SetSyncResponse>(
    `/v1/auth/devices/${encodeURIComponent(deviceId)}/sync`,
    { enabled },
    bearer,
  );
}

// -- Read-side query API --------------------------------------------
// `EventDto` is sourced from the generated OpenAPI schema, but we
// tighten two fields that the codegen emits as optional even though
// the server always populates them (utoipa can't express "nullable
// but required" cleanly, so it falls back to optional + nullable).
// Treating them as required-nullable here matches the runtime wire
// contract and keeps consumers (dashboard, formatters) honest. The
// `payload` slot is widened back to `unknown` because the generated
// `Record<string, never>` is the codegen's stand-in for free-form
// JSON, not an actually-empty object.
export type EventDto = Omit<
  apiSchema['schemas']['EventDto'],
  'event_timestamp' | 'payload'
> & {
  event_timestamp: string | null;
  payload: unknown;
};

// Local mirror of the server's `EventsListResponse` schema. We declare
// it directly rather than aliasing the generated type so we can use
// the locally-tightened `EventDto` (which types `payload: unknown`
// rather than the schema's `Record<string, never>`) and pin
// `next_after` as required-nullable — the server always emits the
// field, with `null` when there's no next page.
export interface ListEventsResponse {
  events: EventDto[];
  next_after: number | null;
}

export async function getSummary(bearer: string): Promise<SummaryResponse> {
  return request<SummaryResponse>('GET', '/v1/me/summary', undefined, bearer);
}

export interface ListEventsParams {
  /** Legacy forward cursor — superseded by after_seq. */
  after?: number;
  /** Older-page cursor: events with seq < before_seq, DESC by seq. */
  before_seq?: number;
  /** Newer-page cursor: events with seq > after_seq, ASC by seq. */
  after_seq?: number;
  /** Filter by event type (validated server-side as [a-z0-9_]{1,64}). */
  event_type?: string;
  /** ISO-8601 lower bound on event_timestamp. */
  since?: string;
  /** ISO-8601 upper bound on event_timestamp. */
  until?: string;
  limit?: number;
}

export async function listEvents(
  bearer: string,
  params: ListEventsParams = {},
): Promise<ListEventsResponse> {
  const qs = new URLSearchParams();
  if (params.after !== undefined) qs.set('after', String(params.after));
  if (params.before_seq !== undefined)
    qs.set('before_seq', String(params.before_seq));
  if (params.after_seq !== undefined)
    qs.set('after_seq', String(params.after_seq));
  if (params.event_type !== undefined && params.event_type !== '')
    qs.set('event_type', params.event_type);
  if (params.since !== undefined && params.since !== '')
    qs.set('since', params.since);
  if (params.until !== undefined && params.until !== '')
    qs.set('until', params.until);
  if (params.limit !== undefined) qs.set('limit', String(params.limit));
  const suffix = qs.toString() ? `?${qs.toString()}` : '';
  const resp = await request<ListEventsResponse>(
    'GET',
    `/v1/me/events${suffix}`,
    undefined,
    bearer,
  );
  // Hide self-explanatory movement events (see `event-filter.ts`).
  // When the caller explicitly asks for one of those types via
  // `event_type`, respect that and skip the filter — they want it.
  if (
    params.event_type &&
    IN_TRANSIT_HIDDEN_TYPES.has(params.event_type)
  ) {
    return resp;
  }
  return { ...resp, events: filterMovementNoise(resp.events) };
}

export type HideToggleResponse =
  apiSchema['schemas']['HideToggleResponse'];

/** Hide one event from shared/public views. Owner-only — the server
 *  filters by claimed_handle = caller. Idempotent: `changed=false`
 *  when the row was already hidden (or doesn't belong to you). */
export async function hideEvent(
  bearer: string,
  seq: number,
): Promise<HideToggleResponse> {
  return request<HideToggleResponse>(
    'POST',
    `/v1/me/events/${seq}/hide`,
    undefined,
    bearer,
  );
}

/** Reverse of {@link hideEvent} — clears `hidden_at`. Same idempotent
 *  semantics. */
export async function unhideEvent(
  bearer: string,
  seq: number,
): Promise<HideToggleResponse> {
  return request<HideToggleResponse>(
    'DELETE',
    `/v1/me/events/${seq}/hide`,
    undefined,
    bearer,
  );
}

// -- Account ---------------------------------------------------------

export async function getMe(bearer: string): Promise<MeResponse> {
  return request<MeResponse>('GET', '/v1/auth/me', undefined, bearer);
}

export async function changePassword(
  bearer: string,
  body: ChangePasswordRequest,
): Promise<ChangePasswordResponse> {
  return postJson<ChangePasswordResponse>('/v1/auth/me/password', body, bearer);
}

export async function resendVerification(
  bearer: string,
): Promise<ResendVerificationResponse> {
  return postJson<ResendVerificationResponse>(
    '/v1/auth/email/resend',
    {},
    bearer,
  );
}

export async function deleteAccount(
  bearer: string,
  body: DeleteAccountRequest,
): Promise<DeleteAccountResponse> {
  return request<DeleteAccountResponse>(
    'DELETE',
    '/v1/auth/me',
    body,
    bearer,
  );
}

// -- Password reset (unauthenticated) -------------------------------
//
// `start` always returns 200 even on miss (anti-enumeration); the
// caller must treat success as "if your address exists, an email is
// on the way." `complete` consumes the token, hashes the new
// password, and the server revokes all device JWTs server-side.

export async function passwordResetStart(
  body: PasswordResetStartRequest,
): Promise<PasswordResetStartResponse> {
  return postJson<PasswordResetStartResponse>(
    '/v1/auth/password/reset/start',
    body,
  );
}

export async function passwordResetComplete(
  body: PasswordResetCompleteRequest,
): Promise<PasswordResetCompleteResponse> {
  return postJson<PasswordResetCompleteResponse>(
    '/v1/auth/password/reset/complete',
    body,
  );
}

// -- Email change ---------------------------------------------------
//
// `start` is authenticated: the active session names a new address,
// the server stashes it on `pending_email` and emails a token there.
// `verify` is unauthenticated because users follow the link straight
// from the inbox; the token is the auth.

export async function emailChangeStart(
  bearer: string,
  body: EmailChangeStartRequest,
): Promise<EmailChangeStartResponse> {
  return postJson<EmailChangeStartResponse>(
    '/v1/auth/email/change/start',
    body,
    bearer,
  );
}

export async function emailChangeVerify(
  body: EmailChangeVerifyRequest,
): Promise<EmailChangeVerifyResponse> {
  return postJson<EmailChangeVerifyResponse>(
    '/v1/auth/email/change/verify',
    body,
  );
}

// -- RSI handle verification ---------------------------------------
//
// `start` issues (or returns a still-valid) verification code. The
// user pastes it into their RSI public bio, then `verify` re-fetches
// the profile and looks for the code. Both endpoints take the user
// bearer; the desktop client doesn't surface bio editing — this
// flow is web-only.

export async function rsiVerifyStart(
  bearer: string,
): Promise<RsiStartResponse> {
  return postJson<RsiStartResponse>('/v1/auth/rsi/start', {}, bearer);
}

export async function rsiVerifyCheck(
  bearer: string,
): Promise<RsiVerifyResponse> {
  return postJson<RsiVerifyResponse>('/v1/auth/rsi/verify', {}, bearer);
}

// -- RSI citizen profile snapshot ----------------------------------
//
// Snapshot of the RSI public profile page (display name, enlistment
// date, badges, bio, primary org). The server caches the result —
// `refreshProfile` re-scrapes RSI (rate-limited to 429 if called
// too eagerly), `getMyProfile` returns the cached snapshot for the
// authenticated user, and `getPublicProfile` returns it for any
// public profile by handle (no auth).

export type ProfileResponse = apiSchema['schemas']['ProfileResponse'];
export type Badge = apiSchema['schemas']['Badge'];

export async function refreshProfile(bearer: string): Promise<ProfileResponse> {
  return postJson<ProfileResponse>('/v1/auth/rsi/profile/refresh', {}, bearer);
}

export async function getMyProfile(bearer: string): Promise<ProfileResponse> {
  return request<ProfileResponse>('GET', '/v1/me/profile', undefined, bearer);
}

/// Hangar snapshot — what the tray client most recently scraped from
/// the user's RSI website pledges page. The server stores the snapshot
/// in `hangar_snapshots`; the tray pushes via POST /v1/me/hangar; nothing
/// on the web actually wrote one here, but the dashboard + settings
/// pages now read it back so the user can see "yes, the tray fed us
/// 17 ships at 14:02" without launching the tray.
export type HangarSnapshot = apiSchema['schemas']['HangarSnapshot'];
export type HangarShip = apiSchema['schemas']['HangarShipSchema'];

/// 404 from the server means "no snapshot yet" — the user either
/// hasn't installed the tray, or hasn't paired it, or hasn't seeded
/// their RSI cookie. We surface that as a typed `null` rather than
/// asking every caller to try/catch a status code; matches the
/// `getCurrentLocation` pattern at `app/dashboard/page.tsx:74-81`.
export async function getMyHangar(
  bearer: string,
): Promise<HangarSnapshot | null> {
  try {
    return await request<HangarSnapshot>(
      'GET',
      '/v1/me/hangar',
      undefined,
      bearer,
    );
  } catch (e) {
    if (e instanceof ApiCallError && e.status === 404) {
      return null;
    }
    throw e;
  }
}

export async function getPublicProfile(
  handle: string,
): Promise<ProfileResponse> {
  return request<ProfileResponse>(
    'GET',
    `/v1/public/u/${encodeURIComponent(handle)}/profile`,
    undefined,
    undefined,
  );
}

// -- RSI org snapshots ---------------------------------------------
//
// Triad mirrors the citizen-profile flow above:
//   * `refreshRsiOrgs` — owner pokes the server to scrape their
//     public RSI org page and persist a snapshot.
//   * `getMyRsiOrgs` — owner reads the most recent snapshot.
//   * `getPublicRsiOrgs` — anyone reads the snapshot for `handle`
//     if visibility allows (the server enforces public/share gating).
//
// All three return `RsiOrgsSnapshot { captured_at, orgs }`. 404 on
// the read endpoints means "no snapshot yet" / "not visible" — the
// callers convert that to a typed null using the same pattern as
// `getMyHangar`.

export type RsiOrgsSnapshot = apiSchema['schemas']['RsiOrgsSnapshot'];
export type RsiOrg = apiSchema['schemas']['RsiOrg'];

export async function refreshRsiOrgs(bearer: string): Promise<RsiOrgsSnapshot> {
  return postJson<RsiOrgsSnapshot>('/v1/auth/rsi/orgs/refresh', {}, bearer);
}

export async function getMyRsiOrgs(
  bearer: string,
): Promise<RsiOrgsSnapshot | null> {
  try {
    return await request<RsiOrgsSnapshot>(
      'GET',
      '/v1/me/rsi-orgs',
      undefined,
      bearer,
    );
  } catch (e) {
    if (e instanceof ApiCallError && e.status === 404) {
      return null;
    }
    throw e;
  }
}

export async function getPublicRsiOrgs(
  handle: string,
): Promise<RsiOrgsSnapshot | null> {
  try {
    return await request<RsiOrgsSnapshot>(
      'GET',
      `/v1/public/u/${encodeURIComponent(handle)}/orgs`,
      undefined,
      undefined,
    );
  } catch (e) {
    if (e instanceof ApiCallError && (e.status === 404 || e.status === 403)) {
      return null;
    }
    throw e;
  }
}

// -- Magic-link sign-in --------------------------------------------
//
// `start` is anti-enumeration: always returns 200 even on miss.
// `redeem` consumes the token and returns an `AuthResponse` —
// possibly with `totp_required: true` if the account has 2FA.

export async function magicLinkStart(
  body: MagicLinkStartRequest,
): Promise<MagicLinkStartResponse> {
  return postJson<MagicLinkStartResponse>('/v1/auth/magic/start', body);
}

export async function magicLinkRedeem(
  body: MagicLinkRedeemRequest,
): Promise<AuthResponse> {
  return postJson<AuthResponse>('/v1/auth/magic/redeem', body);
}

// -- TOTP 2FA ------------------------------------------------------
//
// Setup, confirm, disable, regenerate are authenticated with the
// regular user bearer. `verify-login` is the post-password leg of
// 2FA login: the bearer is the *interim* token returned by /login
// or /magic/redeem when `totp_required` was true.

/** Render a TOTP provisioning URI as a QR, on our own server.
 *
 *  Never hand the provisioning URI to a third-party image service: it
 *  contains the shared secret, and whoever holds that can generate valid
 *  codes indefinitely. This endpoint exists because the URI is small
 *  enough for the enrolment cookie but the QR is not (~9 KB vs a ~4 KB
 *  cookie cap), so the picture is fetched when it is needed.
 *
 *  Returns `null` on failure — enrolment still works via the manually
 *  typed setup key, which is the documented fallback. */
export async function totpQr(provisioningUri: string, bearer: string): Promise<string | null> {
  try {
    const resp = await postJson<{ data_uri: string }>(
      '/v1/auth/totp/qr',
      { provisioning_uri: provisioningUri },
      bearer,
    );
    return resp.data_uri ?? null;
  } catch (e) {
    console.warn('totp qr render failed; manual key only', e);
    return null;
  }
}

export async function totpSetup(bearer: string): Promise<TotpSetupResponse> {
  return postJson<TotpSetupResponse>('/v1/auth/totp/setup', {}, bearer);
}

export async function totpConfirm(
  bearer: string,
  body: TotpConfirmRequest,
): Promise<TotpConfirmResponse> {
  return postJson<TotpConfirmResponse>('/v1/auth/totp/confirm', body, bearer);
}

export async function totpDisable(
  bearer: string,
  body: TotpDisableRequest,
): Promise<TotpDisableResponse> {
  return postJson<TotpDisableResponse>('/v1/auth/totp/disable', body, bearer);
}

export async function totpRegenerateRecovery(
  bearer: string,
  body: RegenerateRecoveryRequest,
): Promise<RegenerateRecoveryResponse> {
  return postJson<RegenerateRecoveryResponse>(
    '/v1/auth/totp/recovery/regenerate',
    body,
    bearer,
  );
}

export async function totpVerifyLogin(
  interimToken: string,
  body: VerifyLoginRequest,
): Promise<AuthResponse> {
  return postJson<AuthResponse>(
    '/v1/auth/totp/verify-login',
    body,
    interimToken,
  );
}

export async function getTimeline(
  bearer: string,
  params: { days?: number } = {},
): Promise<TimelineResponse> {
  const qs = new URLSearchParams();
  if (params.days !== undefined) qs.set('days', String(params.days));
  const suffix = qs.toString() ? `?${qs.toString()}` : '';
  return request<TimelineResponse>(
    'GET',
    `/v1/me/timeline${suffix}`,
    undefined,
    bearer,
  );
}

// -- Metrics aggregates ---------------------------------------------
//
// Powers the /metrics page (4 tabs). Overview reuses getSummary +
// getTimeline; the two helpers below cover the new aggregates.

export type EventTypeBreakdownResponse =
  apiSchema['schemas']['EventTypeBreakdownResponse'];
export type EventTypeStatsDto = apiSchema['schemas']['EventTypeStatsDto'];
export type SessionsResponse = apiSchema['schemas']['SessionsResponse'];
export type SessionDto = apiSchema['schemas']['SessionDto'];

export type MetricsRange = '24h' | '7d' | '30d' | '90d' | 'all';

export async function getMetricsEventTypes(
  bearer: string,
  range: MetricsRange = '30d',
): Promise<EventTypeBreakdownResponse> {
  return request<EventTypeBreakdownResponse>(
    'GET',
    `/v1/me/metrics/event-types?range=${encodeURIComponent(range)}`,
    undefined,
    bearer,
  );
}

export async function getMetricsSessions(
  bearer: string,
  params: { limit?: number; offset?: number } = {},
): Promise<SessionsResponse> {
  const qs = new URLSearchParams();
  if (params.limit !== undefined) qs.set('limit', String(params.limit));
  if (params.offset !== undefined) qs.set('offset', String(params.offset));
  const suffix = qs.toString() ? `?${qs.toString()}` : '';
  return request<SessionsResponse>(
    'GET',
    `/v1/me/metrics/sessions${suffix}`,
    undefined,
    bearer,
  );
}

export type IngestHistoryResponse =
  apiSchema['schemas']['IngestHistoryResponse'];
export type IngestBatchDto = apiSchema['schemas']['IngestBatchDto'];

export async function getIngestHistory(
  bearer: string,
  params: { limit?: number; offset?: number; deviceId?: string } = {},
): Promise<IngestHistoryResponse> {
  const qs = new URLSearchParams();
  if (params.limit !== undefined) qs.set('limit', String(params.limit));
  if (params.offset !== undefined) qs.set('offset', String(params.offset));
  // When deviceId is passed the server clamps to only that device's
  // batches. Omitted → account-wide stream (current default).
  if (params.deviceId) qs.set('device_id', params.deviceId);
  const suffix = qs.toString() ? `?${qs.toString()}` : '';
  return request<IngestHistoryResponse>(
    'GET',
    `/v1/me/ingest-history${suffix}`,
    undefined,
    bearer,
  );
}

// -- Submissions ----------------------------------------------------
//
// Wraps the /v1/submissions endpoints. Voting + flagging are
// per-(user, submission) idempotent on the server side; the toggle
// behaviour comes from passing `vote: false` to retract.

export type SubmissionDto = apiSchema['schemas']['SubmissionDto'];
export type SubmissionListResponse = apiSchema['schemas']['ListResponse'];
export type CreateSubmissionRequest =
  apiSchema['schemas']['CreateSubmissionRequest'];
export type CreateSubmissionResponse =
  apiSchema['schemas']['CreateSubmissionResponse'];
export type VoteRequest = apiSchema['schemas']['VoteRequest'];
export type VoteResponse = apiSchema['schemas']['VoteResponse'];
export type FlagRequest = apiSchema['schemas']['FlagRequest'];
export type FlagResponse = apiSchema['schemas']['FlagResponse'];
export type WithdrawResponse = apiSchema['schemas']['WithdrawResponse'];

export type AdminQueueResponse =
  apiSchema['schemas']['AdminQueueResponse'];
export type AuditEntryDto = apiSchema['schemas']['AuditEntryDto'];
export type AuditListResponse = apiSchema['schemas']['AuditListResponse'];
export type AdminRestrictionDto = apiSchema['schemas']['AdminRestrictionDto'];
export type AdminDeleteUserRequest =
  apiSchema['schemas']['AdminDeleteUserRequest'];
export type DeleteMode = apiSchema['schemas']['DeleteMode'];
export type RestrictionRequest = apiSchema['schemas']['RestrictionRequest'];
export type AdminUserDto = apiSchema['schemas']['AdminUserDto'];
/** Superset returned by GET /v1/admin/users/{id} (detail route only). */
export type AdminUserDetailDto = apiSchema['schemas']['AdminUserDetailDto'];
export type AdminUserDeviceDto = apiSchema['schemas']['AdminUserDeviceDto'];
export type AdminUserEventTypeCountDto =
  apiSchema['schemas']['AdminUserEventTypeCountDto'];
export type AdminUserRetentionDto =
  apiSchema['schemas']['AdminUserRetentionDto'];
export type AdminUserListResponse =
  apiSchema['schemas']['AdminUserListResponse'];
export type GrantRoleRequest = apiSchema['schemas']['GrantRoleRequest'];
export type RoleTransitionResponse =
  apiSchema['schemas']['RoleTransitionResponse'];
export type AdminOrgDto = apiSchema['schemas']['AdminOrgDto'];
export type AdminOrgListResponse =
  apiSchema['schemas']['AdminOrgListResponse'];
export type AdminOrgDeleteResponse =
  apiSchema['schemas']['AdminOrgDeleteResponse'];
export type AdminReferenceCategoryDto =
  apiSchema['schemas']['AdminReferenceCategoryDto'];
export type AdminReferenceCategoriesResponse =
  apiSchema['schemas']['AdminReferenceCategoriesResponse'];
export type AdminReferenceEntryDto =
  apiSchema['schemas']['AdminReferenceEntryDto'];
export type AdminReferenceEntriesResponse =
  apiSchema['schemas']['AdminReferenceEntriesResponse'];
export type SubmissionTransitionResponse =
  apiSchema['schemas']['SubmissionTransitionResponse'];
export type AdminSharingOverview =
  apiSchema['schemas']['AdminSharingOverview'];
export type TopGranter = apiSchema['schemas']['TopGranter'];
export type ScopeHistogram = apiSchema['schemas']['ScopeHistogram'];
export type UserSharingContext =
  apiSchema['schemas']['UserSharingContext'];
export type UserShareEdge =
  apiSchema['schemas']['UserShareEdge'];
export type OrgSharingContext =
  apiSchema['schemas']['OrgSharingContext'];
export type OrgMemberSharingSlice =
  apiSchema['schemas']['OrgMemberSharingSlice'];
export type ReportShareRequest =
  apiSchema['schemas']['ReportShareRequest'];
export type ReportShareResponse =
  apiSchema['schemas']['ReportShareResponse'];
export type ShareReportRowDto =
  apiSchema['schemas']['ShareReportRowDto'];
export type ShareReportListResponse =
  apiSchema['schemas']['ShareReportListResponse'];
export type ResolveReportRequest =
  apiSchema['schemas']['ResolveReportRequest'];

// Task 6 — catalog-gap diagnostic (GET /v1/admin/contracts/gaps). See
// `getAdminContractGaps` below for the fetch wrapper.
export type ContractCatalogGapsResponse =
  apiSchema['schemas']['ContractCatalogGapsResponse'];
export type ContractGapDto = apiSchema['schemas']['ContractGapDto'];

export type SubmissionStatus =
  | 'review'
  | 'accepted'
  | 'shipped'
  | 'rejected'
  | 'withdrawn'
  | 'flagged';

export type SubmissionSort = 'newest' | 'oldest' | 'votes';

export async function listSubmissions(
  bearer: string,
  params: {
    status?: SubmissionStatus;
    mine?: boolean;
    sort?: SubmissionSort;
    limit?: number;
    offset?: number;
  } = {},
): Promise<SubmissionListResponse> {
  const qs = new URLSearchParams();
  if (params.status) qs.set('status', params.status);
  if (params.mine) qs.set('mine', 'true');
  if (params.sort) qs.set('sort', params.sort);
  if (params.limit !== undefined) qs.set('limit', String(params.limit));
  if (params.offset !== undefined) qs.set('offset', String(params.offset));
  const suffix = qs.toString() ? `?${qs.toString()}` : '';
  return request<SubmissionListResponse>(
    'GET',
    `/v1/submissions${suffix}`,
    undefined,
    bearer,
  );
}

export async function getSubmission(
  bearer: string,
  id: string,
): Promise<SubmissionDto> {
  return request<SubmissionDto>(
    'GET',
    `/v1/submissions/${encodeURIComponent(id)}`,
    undefined,
    bearer,
  );
}

export async function createSubmission(
  bearer: string,
  body: CreateSubmissionRequest,
): Promise<CreateSubmissionResponse> {
  return request<CreateSubmissionResponse>(
    'POST',
    '/v1/submissions',
    body,
    bearer,
  );
}

export async function voteOnSubmission(
  bearer: string,
  id: string,
  vote: boolean,
): Promise<VoteResponse> {
  return request<VoteResponse>(
    'POST',
    `/v1/submissions/${encodeURIComponent(id)}/vote`,
    { vote },
    bearer,
  );
}

export async function flagSubmission(
  bearer: string,
  id: string,
  reason?: string,
): Promise<FlagResponse> {
  return request<FlagResponse>(
    'POST',
    `/v1/submissions/${encodeURIComponent(id)}/flag`,
    { reason: reason ?? null },
    bearer,
  );
}

export async function withdrawSubmission(
  bearer: string,
  id: string,
): Promise<WithdrawResponse> {
  return request<WithdrawResponse>(
    'POST',
    `/v1/submissions/${encodeURIComponent(id)}/withdraw`,
    undefined,
    bearer,
  );
}

// -- Admin (moderator + admin) -------------------------------------
//
// All four endpoints below require a staff role (moderator or admin)
// — server-side enforced via `StaffRoleSet::has`. The web client gates
// the /admin route surface on `session.staffRoles` for UX, but never
// trusts the cookie alone for authorization.

export async function getAdminSubmissionQueue(
  bearer: string,
  params: {
    status: 'review' | 'flagged' | 'all';
    limit?: number;
    offset?: number;
  },
): Promise<AdminQueueResponse> {
  const qs = new URLSearchParams();
  qs.set('status', params.status);
  if (params.limit !== undefined) qs.set('limit', String(params.limit));
  if (params.offset !== undefined) qs.set('offset', String(params.offset));
  return request<AdminQueueResponse>(
    'GET',
    `/v1/admin/submissions/queue?${qs.toString()}`,
    undefined,
    bearer,
  );
}

/** Paginated users list for /admin/users. Substring search runs
 *  server-side over claimed_handle OR email. */
export async function getAdminUsers(
  bearer: string,
  params: { q?: string; limit?: number; offset?: number } = {},
): Promise<AdminUserListResponse> {
  const qs = new URLSearchParams();
  if (params.q) qs.set('q', params.q);
  if (params.limit !== undefined) qs.set('limit', String(params.limit));
  if (params.offset !== undefined) qs.set('offset', String(params.offset));
  const suffix = qs.toString() ? `?${qs.toString()}` : '';
  return request<AdminUserListResponse>(
    'GET',
    `/v1/admin/users${suffix}`,
    undefined,
    bearer,
  );
}

/** Detail fetch for a single user in /admin/users/[id]. */
export async function getAdminUser(
  bearer: string,
  id: string,
): Promise<AdminUserDetailDto> {
  return request<AdminUserDetailDto>(
    'GET',
    `/v1/admin/users/${encodeURIComponent(id)}`,
    undefined,
    bearer,
  );
}

/**
 * Admin-only account deletion. Irreversible.
 *
 * `pseudonymise` matches the self-serve path (event rows kept,
 * unlinked); `purge` deletes the events too and removes those rows
 * from recipients' timelines.
 */
export async function deleteAdminUser(
  bearer: string,
  id: string,
  body: AdminDeleteUserRequest,
): Promise<{ deleted: boolean; mode: DeleteMode }> {
  return request<{ deleted: boolean; mode: DeleteMode }>(
    'DELETE',
    `/v1/admin/users/${encodeURIComponent(id)}`,
    body,
    bearer,
  );
}

/** Apply or replace a user's restrictions. Moderator-gated. */
export async function setAdminUserRestrictions(
  bearer: string,
  id: string,
  body: RestrictionRequest,
): Promise<AdminRestrictionDto> {
  return request<AdminRestrictionDto>(
    'PUT',
    `/v1/admin/users/${encodeURIComponent(id)}/restrictions`,
    body,
    bearer,
  );
}

/**
 * Lift a user's restrictions. Moderator-gated.
 *
 * Does NOT restore shares revoked by a suspension -- those grants were
 * deleted, not paused.
 */
export async function clearAdminUserRestrictions(
  bearer: string,
  id: string,
): Promise<{ reinstated: boolean }> {
  return request<{ reinstated: boolean }>(
    'DELETE',
    `/v1/admin/users/${encodeURIComponent(id)}/restrictions`,
    undefined,
    bearer,
  );
}

/** Grant a staff role to a user. Admin-only. Idempotent. */
export async function grantAdminUserRole(
  bearer: string,
  id: string,
  body: GrantRoleRequest,
): Promise<RoleTransitionResponse> {
  return postJson<RoleTransitionResponse>(
    `/v1/admin/users/${encodeURIComponent(id)}/roles`,
    body,
    bearer,
  );
}

/** Revoke a staff role from a user. Admin-only. Idempotent. */
export async function revokeAdminUserRole(
  bearer: string,
  id: string,
  role: 'moderator' | 'admin',
): Promise<RoleTransitionResponse> {
  return request<RoleTransitionResponse>(
    'DELETE',
    `/v1/admin/users/${encodeURIComponent(id)}/roles/${encodeURIComponent(role)}`,
    undefined,
    bearer,
  );
}

/** Paginated orgs list (admin view across ALL orgs). */
export async function getAdminOrgs(
  bearer: string,
  params: { q?: string; limit?: number; offset?: number } = {},
): Promise<AdminOrgListResponse> {
  const qs = new URLSearchParams();
  if (params.q) qs.set('q', params.q);
  if (params.limit !== undefined) qs.set('limit', String(params.limit));
  if (params.offset !== undefined) qs.set('offset', String(params.offset));
  const suffix = qs.toString() ? `?${qs.toString()}` : '';
  return request<AdminOrgListResponse>(
    'GET',
    `/v1/admin/orgs${suffix}`,
    undefined,
    bearer,
  );
}

/** Org detail for the admin console. */
export async function getAdminOrg(
  bearer: string,
  slug: string,
): Promise<AdminOrgDto> {
  return request<AdminOrgDto>(
    'GET',
    `/v1/admin/orgs/${encodeURIComponent(slug)}`,
    undefined,
    bearer,
  );
}

/** Admin force-delete an org. Wipes SpiceDB relationships + the
 *  Postgres row. Admin-only. */
export async function deleteAdminOrg(
  bearer: string,
  slug: string,
): Promise<AdminOrgDeleteResponse> {
  return request<AdminOrgDeleteResponse>(
    'DELETE',
    `/v1/admin/orgs/${encodeURIComponent(slug)}`,
    undefined,
    bearer,
  );
}

/** Per-category summary of the reference_registry. Surfaces row count
 *  + last sync time so admins can spot a stuck cron at a glance. */
export async function getAdminReferenceCategories(
  bearer: string,
): Promise<AdminReferenceCategoriesResponse> {
  return request<AdminReferenceCategoriesResponse>(
    'GET',
    '/v1/admin/reference/categories',
    undefined,
    bearer,
  );
}

/** Paginated entry list for a single category. `q` is a
 *  case-insensitive substring filter on class_name + display_name. */
export async function getAdminReferenceEntries(
  bearer: string,
  category: string,
  params: { q?: string; limit?: number; offset?: number } = {},
): Promise<AdminReferenceEntriesResponse> {
  const qs = new URLSearchParams();
  if (params.q) qs.set('q', params.q);
  if (params.limit !== undefined) qs.set('limit', String(params.limit));
  if (params.offset !== undefined) qs.set('offset', String(params.offset));
  const suffix = qs.toString() ? `?${qs.toString()}` : '';
  return request<AdminReferenceEntriesResponse>(
    'GET',
    `/v1/admin/reference/${encodeURIComponent(category)}${suffix}`,
    undefined,
    bearer,
  );
}

export type ReferenceSyncResponse =
  apiSchema['schemas']['ReferenceSyncResponse'];

/** Kick off a wiki reference sync.
 *
 *  Returns 202 with `started: true` when the worker picked it up, or
 *  409 with `started: false` when one is already queued. The 409 is a
 *  normal outcome, not an error, so it is mapped to a value rather
 *  than thrown — the caller reports it as "already running". */
export async function triggerReferenceSync(
  bearer: string,
): Promise<ReferenceSyncResponse> {
  try {
    return await postJson<ReferenceSyncResponse>(
      '/v1/admin/reference/sync',
      {},
      bearer,
    );
  } catch (e) {
    if (e instanceof ApiCallError && e.status === 409) {
      return {
        started: false,
        detail: 'a reference sync is already queued or running',
      };
    }
    throw e;
  }
}

/**
 * Paginated audit-log fetch for the /admin/audit page. Server is
 * gated on moderator role; the client gates the page on
 * `session.staffRoles` for UX but never trusts the cookie alone.
 *
 * Filters are passed through as querystring params; empty/undefined
 * filters are omitted so the server treats them as "no filter"
 * rather than "filter for empty string".
 */
export async function getAdminAuditLog(
  bearer: string,
  params: {
    actor?: string;
    action?: string;
    since?: string;
    until?: string;
    limit?: number;
    offset?: number;
  } = {},
): Promise<AuditListResponse> {
  const qs = new URLSearchParams();
  if (params.actor) qs.set('actor', params.actor);
  if (params.action) qs.set('action', params.action);
  if (params.since) qs.set('since', params.since);
  if (params.until) qs.set('until', params.until);
  if (params.limit !== undefined) qs.set('limit', String(params.limit));
  if (params.offset !== undefined) qs.set('offset', String(params.offset));
  const suffix = qs.toString() ? `?${qs.toString()}` : '';
  return request<AuditListResponse>(
    'GET',
    `/v1/admin/audit${suffix}`,
    undefined,
    bearer,
  );
}

/**
 * Catalog-gap diagnostic for the /admin/contract-gaps page (Task 7).
 * Surfaces run-observed contract names with no matching row in the
 * published catalog, ranked by occurrence (`run_count` DESC) rather
 * than distinct name count — see `ContractCatalogGapsResponse`'s
 * generated doc for why (Combat Gauntlet is ~5% of distinct gap names
 * in the corpus but 37% of all runs; a name-ranked list would bury
 * the biggest publishing win). Callers must render `gaps` in the
 * order received, not re-sort.
 *
 * Gated server-side on moderator role (`RequireModerator`), same
 * posture as `getAdminAuditLog` above.
 */
export async function getAdminContractGaps(
  bearer: string,
  params: { limit?: number } = {},
): Promise<ContractCatalogGapsResponse> {
  const qs = new URLSearchParams();
  if (params.limit !== undefined) qs.set('limit', String(params.limit));
  const suffix = qs.toString() ? `?${qs.toString()}` : '';
  return request<ContractCatalogGapsResponse>(
    'GET',
    `/v1/admin/contracts/gaps${suffix}`,
    undefined,
    bearer,
  );
}

/**
 * Headline counters + top-20 granters for the /admin/sharing
 * overview page. Replaces the audit-log-window proxy the page used
 * before the dedicated endpoint shipped.
 *
 * Gated server-side on moderator role; the page also redirects on 403
 * for UX.
 */
export async function getAdminSharingOverview(
  bearer: string,
): Promise<AdminSharingOverview> {
  return request<AdminSharingOverview>(
    'GET',
    '/v1/admin/sharing/overview',
    undefined,
    bearer,
  );
}

/**
 * Per-kind distribution of active `share_metadata` rows + per-tab
 * usage for `kind = 'tabs'` rows. Powers the scope-histogram card on
 * /admin/sharing.
 */
export async function getAdminSharingScopeHistogram(
  bearer: string,
): Promise<ScopeHistogram> {
  return request<ScopeHistogram>(
    'GET',
    '/v1/admin/sharing/scope-histogram',
    undefined,
    bearer,
  );
}

/**
 * Audit v2.1 §C — per-user sharing context for the admin user-detail
 * sub-tab. One round-trip returns outbound + inbound shares + open
 * reports involving this user (as reporter and as owner).
 */
export async function getAdminUserSharingContext(
  bearer: string,
  handle: string,
): Promise<UserSharingContext> {
  return request<UserSharingContext>(
    'GET',
    `/v1/admin/sharing/by-user/${encodeURIComponent(handle)}`,
    undefined,
    bearer,
  );
}

/**
 * Per-org sharing context for the /admin/orgs/[slug] Sharing
 * sub-tab. One round-trip returns aggregate per-member share counts
 * plus reports involving any member of the org (as reporter or as
 * owner). Drilldown to the per-user page for full edge detail.
 *
 * Throws ApiCallError(503) when the SpiceDB sidecar is unavailable —
 * org membership lives there, so without it the page can't list
 * members and the caller should render a "service degraded" banner.
 */
export async function getAdminOrgSharingContext(
  bearer: string,
  slug: string,
): Promise<OrgSharingContext> {
  return request<OrgSharingContext>(
    'GET',
    `/v1/admin/sharing/by-org/${encodeURIComponent(slug)}`,
    undefined,
    bearer,
  );
}

/**
 * File a moderation report against a specific (owner, recipient)
 * share. Reporter is taken off the bearer token by the server —
 * the request body does not (and must not) include a reporter
 * field, so spoofing is impossible from the client.
 *
 * Authorization: the auth'd user must be EITHER the owner or the
 * recipient of the share. A third party gets 403.
 * Rate-limited at the server: 5 reports per reporter per 24h.
 */
export async function reportShare(
  bearer: string,
  body: ReportShareRequest,
): Promise<ReportShareResponse> {
  return request<ReportShareResponse>(
    'POST',
    '/v1/share/report',
    body,
    bearer,
  );
}

/**
 * Moderator queue feed. Defaults to `status=open` server-side, so
 * passing no filter returns the unresolved triage queue. Pass
 * `status='all'` for the audit-style view.
 */
export async function getAdminSharingReports(
  bearer: string,
  opts?: { status?: string; limit?: number; offset?: number },
): Promise<ShareReportListResponse> {
  const qs = new URLSearchParams();
  if (opts?.status) qs.set('status', opts.status);
  if (opts?.limit != null) qs.set('limit', String(opts.limit));
  if (opts?.offset != null) qs.set('offset', String(opts.offset));
  const path = qs.toString()
    ? `/v1/admin/sharing/reports?${qs.toString()}`
    : '/v1/admin/sharing/reports';
  return request<ShareReportListResponse>('GET', path, undefined, bearer);
}

/**
 * Moderator triage action. `outcome` must be one of
 * `dismissed | share_revoked | user_suspended`. A second resolve
 * on the same row returns 409 (`already_resolved`) — handle the
 * race by refreshing the queue.
 */
export async function resolveShareReport(
  bearer: string,
  id: string,
  body: ResolveReportRequest,
): Promise<ShareReportRowDto> {
  return request<ShareReportRowDto>(
    'POST',
    `/v1/admin/sharing/reports/${encodeURIComponent(id)}/resolve`,
    body,
    bearer,
  );
}

export async function acceptSubmission(
  bearer: string,
  id: string,
): Promise<SubmissionTransitionResponse> {
  return request<SubmissionTransitionResponse>(
    'POST',
    `/v1/admin/submissions/${encodeURIComponent(id)}/accept`,
    undefined,
    bearer,
  );
}

export async function rejectSubmission(
  bearer: string,
  id: string,
  reason: string,
): Promise<SubmissionTransitionResponse> {
  return request<SubmissionTransitionResponse>(
    'POST',
    `/v1/admin/submissions/${encodeURIComponent(id)}/reject`,
    { reason },
    bearer,
  );
}

export async function dismissSubmissionFlag(
  bearer: string,
  id: string,
): Promise<SubmissionTransitionResponse> {
  return request<SubmissionTransitionResponse>(
    'POST',
    `/v1/admin/submissions/${encodeURIComponent(id)}/dismiss-flag`,
    undefined,
    bearer,
  );
}

// -- Admin parser-submissions (rule-author moderation) ------------

export type AdminParserSubmissionSummary =
  apiSchema['schemas']['AdminSubmissionSummary'];
export type AdminParserSubmissionsListResponse =
  apiSchema['schemas']['AdminSubmissionsListResponse'];
export type AdminParserSubmissionDetail =
  apiSchema['schemas']['AdminSubmissionDetail'];
export type AdminParserSubmissionPatch =
  apiSchema['schemas']['AdminSubmissionPatch'];

export type AdminParserSubmissionStatus =
  | 'pending'
  | 'drafting'
  | 'rule_written'
  | 'dismissed';

/**
 * Paginated parser-submissions list for /admin/parser-submissions.
 *
 * Server sorts by popularity (submitter_count DESC,
 * total_occurrence_count DESC, last_submitted_at DESC) so the
 * moderator sees the highest-impact shapes first. `after` is an
 * opaque cursor — echo back the previous response's `next_after`.
 *
 * `status` defaults to `pending` server-side when omitted. Pass
 * `all` to opt out of bucketing entirely.
 */
export async function getAdminParserSubmissions(
  bearer: string,
  params: {
    status?: AdminParserSubmissionStatus | 'all';
    limit?: number;
    after?: number;
  } = {},
): Promise<AdminParserSubmissionsListResponse> {
  const qs = new URLSearchParams();
  if (params.status) qs.set('status', params.status);
  if (params.limit !== undefined) qs.set('limit', String(params.limit));
  if (params.after !== undefined) qs.set('after', String(params.after));
  const suffix = qs.toString() ? `?${qs.toString()}` : '';
  return request<AdminParserSubmissionsListResponse>(
    'GET',
    `/v1/admin/parser-submissions${suffix}`,
    undefined,
    bearer,
  );
}

export async function getAdminParserSubmission(
  bearer: string,
  id: number,
): Promise<AdminParserSubmissionDetail> {
  return request<AdminParserSubmissionDetail>(
    'GET',
    `/v1/admin/parser-submissions/${id}`,
    undefined,
    bearer,
  );
}

export async function patchAdminParserSubmission(
  bearer: string,
  id: number,
  body: AdminParserSubmissionPatch,
): Promise<AdminParserSubmissionDetail> {
  return request<AdminParserSubmissionDetail>(
    'PATCH',
    `/v1/admin/parser-submissions/${id}`,
    body,
    bearer,
  );
}

export type PublishCommunityBody =
  apiSchema['schemas']['PublishCommunityRequest'];
export type PublishCommunityResponse =
  apiSchema['schemas']['PublishCommunityResponse'];

/**
 * Promote a parser-submission shape into the public community queue.
 * Idempotent server-side: re-publishing the same shape returns the
 * existing row with `already_published: true`. Pass
 * `force_anonymous: true` to override an attributed submitter and
 * publish under the `community` system account instead.
 */
export async function publishSubmissionToCommunity(
  bearer: string,
  id: number,
  body: PublishCommunityBody,
): Promise<PublishCommunityResponse> {
  return request<PublishCommunityResponse>(
    'POST',
    `/v1/admin/parser-submissions/${id}/publish`,
    body,
    bearer,
  );
}

export type AdminParserRuleRow = apiSchema['schemas']['AdminParserRuleRow'];
export type AdminParserRulesListResponse =
  apiSchema['schemas']['AdminParserRulesListResponse'];
export type PublishRuleBody = apiSchema['schemas']['PublishRuleRequest'];
export type PublishRuleResponse = apiSchema['schemas']['PublishRuleResponse'];

/** All published parser rules (enabled + retracted) for /admin/parser-rules. */
export async function getAdminParserRules(
  bearer: string,
): Promise<AdminParserRulesListResponse> {
  return request<AdminParserRulesListResponse>(
    'GET',
    '/v1/admin/parser-rules',
    undefined,
    bearer,
  );
}

/**
 * Upsert a parser rule into the served manifest. `enabled: false`
 * retracts a previously-published rule.
 */
export async function publishAdminParserRule(
  bearer: string,
  body: PublishRuleBody,
): Promise<PublishRuleResponse> {
  return request<PublishRuleResponse>(
    'POST',
    '/v1/admin/parser-rules',
    body,
    bearer,
  );
}

export type PlayerFactsResponse = apiSchema['schemas']['FactsResponse'];
export type PlayerFact = apiSchema['schemas']['Fact'];

/**
 * Player Facts for the signed-in player (#368).
 *
 * Deliberately takes no range: scope belongs to each fact, not to the
 * request. Re-scoping a lifetime observation to the dashboard's 24h range is
 * the defect that made the commerce and corridor widgets quietly wrong.
 */
export async function getPlayerFacts(
  bearer: string,
): Promise<PlayerFactsResponse> {
  return request<PlayerFactsResponse>('GET', '/v1/me/facts', undefined, bearer);
}

export type ParserHealthResponse =
  apiSchema['schemas']['ParserHealthResponse'];
export type ParserHealthFinding = apiSchema['schemas']['StoredFinding'];
/** A finding plus the unknown log tags that appeared when it went dark. */
export type ParserHealthFindingView = apiSchema['schemas']['FindingView'];
export type ParserHealthTagCandidate = apiSchema['schemas']['TagCandidate'];
export type ParserHealthRun = apiSchema['schemas']['HealthRun'];

/**
 * Detector state for /admin/parser-health: the last pass plus every
 * finding. `last_run` is deliberately part of the same payload — a stale
 * or absent run matters more than an empty findings list, and the two
 * must never be mistaken for each other.
 */
export async function getAdminParserHealth(
  bearer: string,
): Promise<ParserHealthResponse> {
  return request<ParserHealthResponse>(
    'GET',
    '/v1/admin/parser-health',
    undefined,
    bearer,
  );
}

/** Silence a finding for a type that is legitimately dead. */
export async function acknowledgeParserHealthFinding(
  bearer: string,
  eventType: string,
  note?: string,
): Promise<void> {
  await request<void>(
    'POST',
    `/v1/admin/parser-health/${encodeURIComponent(eventType)}/acknowledge`,
    { note: note ?? null },
    bearer,
  );
}

/** Close a finding by hand. The detector also auto-resolves on recovery. */
export async function resolveParserHealthFinding(
  bearer: string,
  eventType: string,
): Promise<void> {
  await request<void>(
    'POST',
    `/v1/admin/parser-health/${encodeURIComponent(eventType)}/resolve`,
    undefined,
    bearer,
  );
}

export type InferenceRuleDto = apiSchema['schemas']['InferenceRuleDto'];
export type PublishInferenceRuleRequest =
  apiSchema['schemas']['PublishInferenceRuleRequest'];
export type PublishInferenceRuleResponse =
  apiSchema['schemas']['PublishInferenceRuleResponse'];
export type AdminInferenceRuleRow =
  apiSchema['schemas']['AdminInferenceRuleRow'];
export type AdminInferenceRulesListResponse =
  apiSchema['schemas']['AdminInferenceRulesListResponse'];
export type EventTypesResponse = apiSchema['schemas']['EventTypesResponse'];

/** All published inference rules (enabled + retracted) for /admin/inference-rules. */
export async function getAdminInferenceRules(
  bearer: string,
): Promise<AdminInferenceRulesListResponse> {
  return request<AdminInferenceRulesListResponse>(
    'GET',
    '/v1/admin/parser-inference-rules',
    undefined,
    bearer,
  );
}

/**
 * Upsert an inference rule into the served manifest. `enabled: false`
 * retracts a previously-published rule.
 */
export async function publishAdminInferenceRule(
  bearer: string,
  body: PublishInferenceRuleRequest,
): Promise<PublishInferenceRuleResponse> {
  return request<PublishInferenceRuleResponse>(
    'POST',
    '/v1/admin/parser-inference-rules',
    body,
    bearer,
  );
}

/** Known event-type keys, for populating trigger/emits pickers in the rule editor. */
export async function getAdminEventTypes(
  bearer: string,
): Promise<EventTypesResponse> {
  return request<EventTypesResponse>(
    'GET',
    '/v1/admin/event-types',
    undefined,
    bearer,
  );
}

// -- Supporter (donate) --------------------------------------------
//
// Read-only for now. The actual checkout / webhook flow depends on
// Revolut Business credentials being provisioned (see
// docs/REVOLUT-INTEGRATION-PLAN.md). The read endpoint already exists
// so the supporter pill on the profile / settings pages can light up
// against any manually-set row.

export type SupporterStatusDto =
  apiSchema['schemas']['SupporterStatusDto'];

export type SupporterState = 'none' | 'active' | 'lapsed';

export async function getSupporterStatus(
  bearer: string,
): Promise<SupporterStatusDto> {
  return request<SupporterStatusDto>(
    'GET',
    '/v1/me/supporter',
    undefined,
    bearer,
  );
}

// -- Location: where the user currently is in-game ---------------
//
// Backed by `GET /v1/me/location/current` on the server. Returns 204
// (translated to `null` here) when the most recent location-bearing
// event is older than the staleness window (90 minutes) — the UI
// uses null as the "no recent activity" signal.

export type ResolvedLocation = apiSchema['schemas']['ResolvedLocation'];
export type CurrentLocationResponse =
  apiSchema['schemas']['CurrentLocationResponse'];

export async function getCurrentLocation(
  bearer: string,
): Promise<ResolvedLocation | null> {
  // request<T>() already returns undefined on 204; we narrow to null
  // here so callers don't accidentally read fields off undefined.
  const resp = (await request<CurrentLocationResponse | undefined>(
    'GET',
    '/v1/me/location/current',
    undefined,
    bearer,
  )) as CurrentLocationResponse | undefined;
  return resp?.location ?? null;
}

export type TraceResponse = apiSchema['schemas']['TraceResponse'];
export type TraceEntry = apiSchema['schemas']['TraceEntry'];
export type BreakdownResponse = apiSchema['schemas']['BreakdownResponse'];
export type BreakdownEntry = apiSchema['schemas']['BreakdownEntry'];
export type StatsBucket = apiSchema['schemas']['StatsBucket'];
export type CombatStatsResponse =
  apiSchema['schemas']['CombatStatsResponse'];
export type TravelStatsResponse =
  apiSchema['schemas']['TravelStatsResponse'];
export type LoadoutStatsResponse =
  apiSchema['schemas']['LoadoutStatsResponse'];
export type StabilityStatsResponse =
  apiSchema['schemas']['StabilityStatsResponse'];
export type PlaytimeStatsResponse =
  apiSchema['schemas']['PlaytimeStatsResponse'];
export type LocationsStatsResponse =
  apiSchema['schemas']['LocationsStatsResponse'];
export type LivesResponse = apiSchema['schemas']['LivesResponse'];
export type LifeRow = apiSchema['schemas']['LifeRow'];
export type FleetResponse = apiSchema['schemas']['FleetResponse'];
export type DockingResponse = apiSchema['schemas']['DockingResponse'];
export type RoutesResponse = apiSchema['schemas']['RoutesResponse'];
export type RouteRow = apiSchema['schemas']['RouteRow'];
export type ObjectivesResponse = apiSchema['schemas']['ObjectivesResponse'];
export type ContractsResponse = apiSchema['schemas']['ContractsResponse'];
export type ContractRunRow = apiSchema['schemas']['ContractRunRow'];
export type ContractStepRow = apiSchema['schemas']['ContractStepRow'];
export type SpendResponse = apiSchema['schemas']['SpendResponse'];
export type LoadoutActivityResponse =
  apiSchema['schemas']['LoadoutActivityResponse'];
export type LoadoutItemRow = apiSchema['schemas']['LoadoutItemRow'];

export async function getLocationTrace(
  bearer: string,
  hours: number = 24,
): Promise<TraceResponse> {
  return request<TraceResponse>(
    'GET',
    `/v1/me/location/trace?hours=${hours}`,
    undefined,
    bearer,
  );
}

export async function getLocationBreakdown(
  bearer: string,
  hours: number = 24 * 7,
): Promise<BreakdownResponse> {
  return request<BreakdownResponse>(
    'GET',
    `/v1/me/location/breakdown?hours=${hours}`,
    undefined,
    bearer,
  );
}

export async function getCombatStats(
  bearer: string,
  hours: number = 24 * 30,
): Promise<CombatStatsResponse> {
  return request<CombatStatsResponse>(
    'GET',
    `/v1/me/stats/combat?hours=${hours}`,
    undefined,
    bearer,
  );
}

export async function getTravelStats(
  bearer: string,
  hours: number = 24 * 30,
): Promise<TravelStatsResponse> {
  return request<TravelStatsResponse>(
    'GET',
    `/v1/me/stats/travel?hours=${hours}`,
    undefined,
    bearer,
  );
}

export async function getLoadoutStats(
  bearer: string,
  hours: number = 24 * 30,
): Promise<LoadoutStatsResponse> {
  return request<LoadoutStatsResponse>(
    'GET',
    `/v1/me/stats/loadout?hours=${hours}`,
    undefined,
    bearer,
  );
}

export type CommerceTxKind = 'shop' | 'commodity_buy' | 'commodity_sell';
export type CommerceTxStatus =
  | 'pending'
  | 'confirmed'
  | 'rejected'
  | 'timed_out'
  | 'submitted';

// Drift fix #5: switch the field shapes to come from the generated
// schema (server now registers CommerceTransactionDto +
// CommerceRecentResponse in openapi.rs). The two `kind` / `status`
// fields stay re-typed to the local literal unions because the
// server returns plain `String` — narrowing on the client side
// preserves call-site exhaustiveness checks (e.g.
// `formatCommerceStatus` in journey/page.tsx). Trade-off: a new
// kind/status variant added on the server will silently fall outside
// the union here until this file is updated. Long-term fix is to
// turn the Rust types into enums; until then, this comment + the
// narrowing is the contract.
export type CommerceTransaction = Omit<
  apiSchema['schemas']['CommerceTransactionDto'],
  'kind' | 'status'
> & {
  kind: CommerceTxKind;
  status: CommerceTxStatus;
};
// `CommerceRecentResponse` mirrors the server schema but with the
// inner array re-typed to the narrowed `CommerceTransaction` so the
// kind/status unions reach call sites.
export interface CommerceRecentResponse {
  transactions: CommerceTransaction[];
}

export async function getCommerceRecent(
  bearer: string,
  limit: number = 100,
  windowSecs: number = 30,
  /** Optional time-range filter in hours. When omitted the server
   *  pulls the last ~1000 events of any type and filters to commerce
   *  variants in-process (legacy behavior). When set, only events
   *  newer than `now - hours` are considered. Matches the journey
   *  range chip selector. */
  hours?: number,
): Promise<CommerceRecentResponse> {
  const params = new URLSearchParams({
    limit: String(limit),
    window_secs: String(windowSecs),
  });
  if (hours !== undefined) {
    params.set('hours', String(hours));
  }
  return request<CommerceRecentResponse>(
    'GET',
    `/v1/me/commerce/recent?${params.toString()}`,
    undefined,
    bearer,
  );
}

export interface BiggestTradeResponse {
  /** Quantity of the biggest confirmed trade; null when there are none. */
  quantity: number | null;
  /** Item of that trade, when the event carried one. */
  item: string | null;
}

/**
 * The caller's largest CONFIRMED commerce purchase by quantity over their
 * FULL history (F9) — a server aggregate that replaces the old client scan
 * of the 500-capped recent-commerce list (which could miss a big trade
 * outside the recent window). Me-scoped (owner-only). Hand-typed: the
 * endpoint is intentionally not in the OpenAPI spec (mirrors getRecords).
 */
export async function getBiggestTrade(
  bearer: string,
): Promise<BiggestTradeResponse> {
  return request<BiggestTradeResponse>(
    'GET',
    '/v1/me/stats/biggest-trade',
    undefined,
    bearer,
  );
}

export async function getStabilityStats(
  bearer: string,
  hours: number = 24 * 30,
): Promise<StabilityStatsResponse> {
  return request<StabilityStatsResponse>(
    'GET',
    `/v1/me/stats/stability?hours=${hours}`,
    undefined,
    bearer,
  );
}

export async function getPlaytime(
  bearer: string,
  hours?: number,
  allTime?: boolean,
): Promise<PlaytimeStatsResponse> {
  // all_time takes precedence over hours and aggregates over all
  // recorded history (the server caps the hours window at 1 year).
  const qs = allTime
    ? '?all_time=true'
    : hours !== undefined
      ? `?hours=${hours}`
      : '';
  return request<PlaytimeStatsResponse>(
    'GET',
    `/v1/me/stats/playtime${qs}`,
    undefined,
    bearer,
  );
}

/**
 * All-time "records" for the caller, computed server-side over the FULL
 * event history (audit F9) — replaces the widget's former client-side,
 * fetch-capped computation. Me-scoped (the authenticated caller's own
 * data); there is no handle-scoped variant, so the records widget uses
 * this only on the owner path.
 *
 * Hand-typed: this endpoint isn't in the generated OpenAPI schema yet.
 * Add `#[utoipa::path]` on `stats_records` + regen the TS client to move
 * to the generated type.
 */
export type RecordsWindow = {
  hours: number;
  longest_session_secs: number;
  busiest_session_events: number;
  longest_survival_streak_secs: number;
  deadliest_session_deaths: number;
};

export type RecordsResponse = {
  longest_session_secs: number;
  busiest_session_events: number;
  longest_survival_streak_secs: number;
  deadliest_session_deaths: number;
  /**
   * Present only when `getRecords` is called with an `hours` window: the
   * same records over just the trailing N hours. Omitted for the default
   * all-time request.
   */
  window?: RecordsWindow;
};

export async function getRecords(
  bearer: string,
  hours?: number,
): Promise<RecordsResponse> {
  const qs = hours !== undefined ? `?hours=${hours}` : '';
  return request<RecordsResponse>(
    'GET',
    `/v1/me/stats/records${qs}`,
    undefined,
    bearer,
  );
}

export async function getLocationsVisited(
  bearer: string,
  hours?: number,
): Promise<LocationsStatsResponse> {
  const qs = hours !== undefined ? `?hours=${hours}` : '';
  return request<LocationsStatsResponse>(
    'GET',
    `/v1/me/stats/locations${qs}`,
    undefined,
    bearer,
  );
}

/**
 * Per-life ("character life") breakdown for the caller: headline
 * survival stats (longest/mean life, deaths-per-session, crash-ended
 * count) plus the 50 most-recent lives. Server aggregate computed over
 * the FULL event history via the character-life FSM. Me-scoped
 * (owner-only) — there is no handle-scoped variant.
 *
 * Unlike `getRecords`/`getBiggestTrade` above, `stats_lives` IS
 * registered in the OpenAPI spec, so this is schema-typed rather than
 * hand-typed.
 */
export async function getLives(
  bearer: string,
  hours?: number,
): Promise<LivesResponse> {
  const qs = hours !== undefined ? `?hours=${hours}` : '';
  return request<LivesResponse>(
    'GET',
    `/v1/me/stats/lives${qs}`,
    undefined,
    bearer,
  );
}

/**
 * "Ships you fly" — top vehicle classes by `quantum_target_selected`
 * trip count, ranked desc. Me-scoped (owner-only). Honest caveat: this
 * reflects quantum-travel usage, not the caller's full owned/hangar
 * fleet (StarStats never fetches hangar/pledge data server-side —
 * see docs/ENGINEERING.md "Architecture Invariants").
 */
export async function getFleet(
  bearer: string,
  hours?: number,
): Promise<FleetResponse> {
  const qs = hours !== undefined ? `?hours=${hours}` : '';
  return request<FleetResponse>(
    'GET',
    `/v1/me/stats/fleet${qs}`,
    undefined,
    bearer,
  );
}

/**
 * "Docking profile" — hangar-vs-pad split plus ship-size distribution
 * of stow events, ranked from `landing_zone_docking`/comparable
 * payloads. Me-scoped (owner-only), numeric-only aggregate (no ship
 * identity), mirrors `getFleet` above.
 */
export async function getDocking(
  bearer: string,
  hours?: number,
): Promise<DockingResponse> {
  const qs = hours !== undefined ? `?hours=${hours}` : '';
  return request<DockingResponse>(
    'GET',
    `/v1/me/stats/docking${qs}`,
    undefined,
    bearer,
  );
}

/**
 * Correlation surfaces (reparse-gated). Each mirrors `getFleet`/`getDocking`
 * — me-scoped, owner-only aggregates over the newer parser event types:
 *   - routes: top quantum destinations (`quantum_route`)
 *   - objectives: mission-objective completion (`mission_objective`)
 *   - spend: kiosk spending totals (`shop_buy_request.price`)
 *   - loadout-activity: gear equip/store churn (`item_equip_change`)
 */
export async function getRoutes(
  bearer: string,
  hours?: number,
): Promise<RoutesResponse> {
  const qs = hours !== undefined ? `?hours=${hours}` : '';
  return request<RoutesResponse>('GET', `/v1/me/stats/routes${qs}`, undefined, bearer);
}

export async function getObjectives(
  bearer: string,
  hours?: number,
): Promise<ObjectivesResponse> {
  const qs = hours !== undefined ? `?hours=${hours}` : '';
  return request<ObjectivesResponse>(
    'GET',
    `/v1/me/stats/objectives${qs}`,
    undefined,
    bearer,
  );
}

export async function getContracts(
  bearer: string,
  hours?: number,
  includeSteps?: boolean,
): Promise<ContractsResponse> {
  const qs = new URLSearchParams();
  if (hours !== undefined) qs.set('hours', String(hours));
  // Opt-in only: the server leaves every run's `steps` as an empty array
  // unless this is set (cheap default for callers, like the `contracts`
  // widget, that only need the run-level counts). An empty `steps` on a
  // response that DID pass `include_steps=true` means the run truly has
  // no steps; on a response that didn't, it means "not requested" — see
  // `ContractRunRow`'s doc. Callers must not conflate the two.
  if (includeSteps) qs.set('include_steps', 'true');
  const suffix = qs.toString();
  return request<ContractsResponse>(
    'GET',
    `/v1/me/stats/contracts${suffix ? `?${suffix}` : ''}`,
    undefined,
    bearer,
  );
}

export async function getSpend(
  bearer: string,
  hours?: number,
): Promise<SpendResponse> {
  const qs = hours !== undefined ? `?hours=${hours}` : '';
  return request<SpendResponse>('GET', `/v1/me/stats/spend${qs}`, undefined, bearer);
}

export async function getLoadoutActivity(
  bearer: string,
): Promise<LoadoutActivityResponse> {
  return request<LoadoutActivityResponse>(
    'GET',
    '/v1/me/stats/loadout-activity',
    undefined,
    bearer,
  );
}

// -- Donate (Revolut Business hosted checkout) --------------------
//
// Wave 9. The server returns 503 `not_configured` when REVOLUT_API_KEY
// is unset, so the donate page renders a "coming soon" panel rather
// than a checkout button in that environment. The tier list is static
// (server-side const) but we fetch it through the API so future
// price-list edits don't require a frontend rebuild.

export type TierDto = apiSchema['schemas']['TierDto'];
export type TierListResponse = apiSchema['schemas']['TierListResponse'];
export type CheckoutRequest = apiSchema['schemas']['CheckoutRequest'];
export type CheckoutResponse = apiSchema['schemas']['CheckoutResponse'];

export async function listDonateTiers(): Promise<TierListResponse> {
  return request<TierListResponse>(
    'GET',
    '/v1/donate/tiers',
    undefined,
    undefined,
  );
}

export async function startDonateCheckout(
  bearer: string,
  body: CheckoutRequest,
): Promise<CheckoutResponse> {
  return request<CheckoutResponse>('POST', '/v1/donate/checkout', body, bearer);
}

// -- Sharing + visibility -------------------------------------------
//
// Server endpoints live in `crates/starstats-server/src/sharing_routes.rs`.
// Helpers here are thin wrappers that surface the generated schema
// types. The public read endpoints (`/v1/public/*`) are unauthenticated;
// the friend read endpoints (`/v1/u/*`) take a bearer.

export type VisibilityRequest = apiSchema['schemas']['VisibilityRequest'];
export type VisibilityResponse = apiSchema['schemas']['VisibilityResponse'];
export type ShareRequest = apiSchema['schemas']['ShareRequest'];
export type ShareResponse = apiSchema['schemas']['ShareResponse'];
export type RevokeShareResponse =
  apiSchema['schemas']['RevokeShareResponse'];
export type ListSharesResponse = apiSchema['schemas']['ListSharesResponse'];
export type ShareEntry = apiSchema['schemas']['ShareEntry'];
/**
 * Per-share scope clamp — wire-level shape generated from
 * `sharing_routes::ShareScope`. `null` (or omitted) means "full
 * manifest", the legacy default every pre-W3 share already has.
 */
export type ShareScope = apiSchema['schemas']['ShareScope'];
export type ListSharedWithMeResponse =
  apiSchema['schemas']['ListSharedWithMeResponse'];
export type SharedWithMeEntry =
  apiSchema['schemas']['SharedWithMeEntry'];
export type PublicSummaryResponse =
  apiSchema['schemas']['PublicSummaryResponse'];
export type PublicTimelineResponse =
  apiSchema['schemas']['PublicTimelineResponse'];

export async function getVisibility(
  bearer: string,
): Promise<VisibilityResponse> {
  return request<VisibilityResponse>(
    'GET',
    '/v1/me/visibility',
    undefined,
    bearer,
  );
}

// Public-profile view counters (Piece 2 of public-profile UX).
//
// `getProfileViews` powers the /sharing "Profile views" card. The
// endpoint reads the bearer-token claim for the handle; we never
// pass it explicitly so a misconfigured caller can't fish for other
// owners' counters.
export type ProfileViewStats = apiSchema['schemas']['ProfileViewStats'];
export type ProfileViewDay = apiSchema['schemas']['ProfileViewDay'];
export type ProfileViewTotals = apiSchema['schemas']['ProfileViewTotals'];
export type ProfileViewSource = apiSchema['schemas']['ProfileViewSource'];

export async function getProfileViews(
  bearer: string,
  options: { days?: number } = {},
): Promise<ProfileViewStats> {
  const params = new URLSearchParams();
  if (options.days !== undefined) {
    params.set('days', String(options.days));
  }
  const qs = params.toString();
  const path = qs === '' ? '/v1/me/profile-views' : `/v1/me/profile-views?${qs}`;
  return request<ProfileViewStats>('GET', path, undefined, bearer);
}

export async function setVisibility(
  bearer: string,
  isPublic: boolean,
  /**
   * Piece 4 — `/discover` listing opt-out. `undefined` means "leave
   * the current value untouched" (the legacy single-arg call site
   * keeps working unchanged); `true`/`false` writes the new value
   * through the same endpoint that flips the SpiceDB public toggle.
   */
  listingOptOut?: boolean,
): Promise<VisibilityResponse> {
  return postJson<VisibilityResponse>(
    '/v1/me/visibility',
    {
      public: isPublic,
      listing_opt_out: listingOptOut,
    } satisfies VisibilityRequest,
    bearer,
  );
}

export async function listShares(bearer: string): Promise<ListSharesResponse> {
  return request<ListSharesResponse>(
    'GET',
    '/v1/me/shares',
    undefined,
    bearer,
  );
}

/**
 * Inbound side of per-user sharing: the owners who have granted
 * the caller view access to their stats_record. Mirrors
 * `listShares` (outbound) but on the receiving end. Org-mediated
 * shares aren't enumerated here — those come from /v1/orgs/me +
 * the per-org detail page.
 */
export async function listSharedWithMe(
  bearer: string,
): Promise<ListSharedWithMeResponse> {
  return request<ListSharedWithMeResponse>(
    'GET',
    '/v1/me/shared-with-me',
    undefined,
    bearer,
  );
}

export async function addShare(
  bearer: string,
  recipientHandle: string,
  options: {
    expiresAt?: string | null;
    note?: string | null;
    /**
     * Per-share scope clamp. `null` or omitted is the legacy
     * "full manifest" default. The server normalises `kind="full"`
     * back to NULL so re-grants from a UI that always sends a scope
     * can still clear it.
     */
    scope?: ShareScope | null;
  } = {},
): Promise<ShareResponse> {
  const body: ShareRequest = {
    recipient_handle: recipientHandle,
  };
  // Only include the optional fields when set so the server doesn't
  // see explicit nulls — the Rust handler treats absence and null
  // the same way, but absence is the canonical "no expiry / no note"
  // shape that round-trips cleanly with #[serde(default)].
  if (options.expiresAt) body.expires_at = options.expiresAt;
  if (options.note) body.note = options.note;
  if (options.scope) body.scope = options.scope;
  return postJson<ShareResponse>('/v1/me/share', body, bearer);
}

export async function removeShare(
  bearer: string,
  recipientHandle: string,
): Promise<RevokeShareResponse> {
  return request<RevokeShareResponse>(
    'DELETE',
    `/v1/me/share/${encodeURIComponent(recipientHandle)}`,
    undefined,
    bearer,
  );
}

export async function getPublicSummary(
  handle: string,
): Promise<PublicSummaryResponse> {
  return request<PublicSummaryResponse>(
    'GET',
    `/v1/public/${encodeURIComponent(handle)}/summary`,
    undefined,
    undefined,
  );
}

export async function getPublicTimeline(
  handle: string,
  days?: number,
): Promise<PublicTimelineResponse> {
  const qs = new URLSearchParams();
  if (days !== undefined) qs.set('days', String(days));
  const suffix = qs.toString() ? `?${qs.toString()}` : '';
  return request<PublicTimelineResponse>(
    'GET',
    `/v1/public/${encodeURIComponent(handle)}/timeline${suffix}`,
    undefined,
    undefined,
  );
}

/**
 * Audit v2.1 §B1 — owner-side preview of own data through a
 * given scope clamp. Returns the same shape as the public summary
 * endpoint so the preview page can reuse public-render components.
 * `scopeJson` is the URL-encoded JSON body of `ShareScope`; empty
 * = full manifest (no clamp).
 */
export async function previewShareSummary(
  bearer: string,
  scopeJson: string | null,
): Promise<PublicSummaryResponse> {
  const qs = new URLSearchParams();
  if (scopeJson) qs.set('scope', scopeJson);
  const suffix = qs.toString() ? `?${qs.toString()}` : '';
  return request<PublicSummaryResponse>(
    'GET',
    `/v1/me/preview-share/summary${suffix}`,
    undefined,
    bearer,
  );
}

export async function previewShareTimeline(
  bearer: string,
  scopeJson: string | null,
  days?: number,
): Promise<PublicTimelineResponse> {
  const qs = new URLSearchParams();
  if (scopeJson) qs.set('scope', scopeJson);
  if (days !== undefined) qs.set('days', String(days));
  const suffix = qs.toString() ? `?${qs.toString()}` : '';
  return request<PublicTimelineResponse>(
    'GET',
    `/v1/me/preview-share/timeline${suffix}`,
    undefined,
    bearer,
  );
}

export async function getFriendSummary(
  bearer: string,
  handle: string,
): Promise<PublicSummaryResponse> {
  return request<PublicSummaryResponse>(
    'GET',
    `/v1/u/${encodeURIComponent(handle)}/summary`,
    undefined,
    bearer,
  );
}

export async function getFriendTimeline(
  bearer: string,
  handle: string,
  days?: number,
): Promise<PublicTimelineResponse> {
  const qs = new URLSearchParams();
  if (days !== undefined) qs.set('days', String(days));
  const suffix = qs.toString() ? `?${qs.toString()}` : '';
  return request<PublicTimelineResponse>(
    'GET',
    `/v1/u/${encodeURIComponent(handle)}/timeline${suffix}`,
    undefined,
    bearer,
  );
}

/** Paired commerce transactions for a friend's profile — gated by
 *  the owner's `share_metadata.scope.allow_widgets`/`deny_widgets`
 *  for the `economy` widget (Plan 3b A.2). Same response shape as
 *  `getCommerceRecent` so the economy widget can render either path
 *  through one component. Treat 404 as "not shared" / "widget
 *  denied" — the widget's catch converts it into a `null` render. */
export async function getFriendCommerceRecent(
  bearer: string,
  handle: string,
  limit: number = 100,
  windowSecs: number = 30,
  hours?: number,
): Promise<CommerceRecentResponse> {
  const params = new URLSearchParams({
    limit: String(limit),
    window_secs: String(windowSecs),
  });
  if (hours !== undefined) {
    params.set('hours', String(hours));
  }
  return request<CommerceRecentResponse>(
    'GET',
    `/v1/u/${encodeURIComponent(handle)}/commerce/recent?${params.toString()}`,
    undefined,
    bearer,
  );
}

// -- Per-event session timeline (share_event_timeline gated) --------
//
// Server endpoints live in
// `crates/starstats-server/src/event_timeline.rs`. Both require a
// bearer token; the server rejects with 403
// `share_event_timeline_not_granted` when the caller has no active
// share-grant with the toggle set. The owner viewing their own
// timeline is always permitted (case-insensitive handle match).

export type SessionSummary = apiSchema['schemas']['SessionSummary'];
export type SessionsListResponse =
  apiSchema['schemas']['SessionsListResponse'];

/**
 * Structural mirror of the server's `SessionEventsResponse`. The
 * generated OpenAPI schema names the same shape
 * `SessionEventsResponseSchema` (a separate type so utoipa can derive
 * `ToSchema` on a wire-compatible struct) — we re-state it here so
 * the consumer side stays clean of the `Schema` suffix.
 */
export interface SessionEventsResponse {
  session_id: string;
  events: EventEnvelopeFromGen[];
  next_after: string | null;
}

type EventEnvelopeFromGen = apiSchema['schemas']['EventEnvelopeSchema'];

export async function getSessions(
  bearer: string,
  handle: string,
  hours?: number,
): Promise<SessionsListResponse> {
  const qs = hours !== undefined ? `?hours=${hours}` : '';
  return request<SessionsListResponse>(
    'GET',
    `/v1/users/${encodeURIComponent(handle)}/sessions${qs}`,
    undefined,
    bearer,
  );
}

export interface UserPlaytimeResponse {
  total_playtime_secs: number;
  session_count: number;
}

/**
 * Handle-scoped all-time playtime + session-count aggregate (F9). Gated
 * by the SAME `share_event_timeline` grant as {@link getSessions} — a 4xx
 * for disallowed access, which the Sessions widget treats as "no lifetime
 * aggregate" and falls back to the capped list. Lets a VISITOR show true
 * lifetime totals instead of silently undercounting from the 50-capped
 * session list. Hand-typed: the endpoint is intentionally not in the
 * OpenAPI spec (mirrors getRecords/RecordsResponse), so there is no
 * generated type to import.
 */
export async function getUserPlaytime(
  bearer: string,
  handle: string,
  hours?: number,
): Promise<UserPlaytimeResponse> {
  const qs = hours !== undefined ? `?hours=${hours}` : '';
  return request<UserPlaytimeResponse>(
    'GET',
    `/v1/users/${encodeURIComponent(handle)}/stats/playtime${qs}`,
    undefined,
    bearer,
  );
}

export async function getSessionEvents(
  bearer: string,
  handle: string,
  sessionId: string,
  opts: { after?: string; limit?: number } = {},
): Promise<SessionEventsResponse> {
  const qs = new URLSearchParams();
  // Empty-string `after` is treated identically to "no cursor" —
  // the wire only encodes a cursor when one was actually supplied.
  // Matches the guard used by `getUserEntities` / `getEntityHistory`.
  if (opts.after !== undefined && opts.after !== '') qs.set('after', opts.after);
  if (opts.limit !== undefined) qs.set('limit', String(opts.limit));
  const suffix = qs.toString() ? `?${qs.toString()}` : '';
  const resp = await request<SessionEventsResponse>(
    'GET',
    `/v1/users/${encodeURIComponent(handle)}/sessions/${encodeURIComponent(sessionId)}/events${suffix}`,
    undefined,
    bearer,
  );
  // Hide self-explanatory movement events (see `event-filter.ts`).
  return { ...resp, events: filterMovementNoise(resp.events) };
}

// -- Cross-session entity rollup (share_event_timeline gated) -------
//
// Server endpoints live in
// `crates/starstats-server/src/entity_rollup.rs`. Auth posture
// mirrors `event_timeline`: owner self-access or an active grant
// with `share_event_timeline = TRUE`. 403 → caller has no grant.

export type EntitySummary = apiSchema['schemas']['EntitySummary'];
export type EntitiesListResponse =
  apiSchema['schemas']['EntitiesListResponse'];
export type EntitySessionBucket =
  apiSchema['schemas']['EntitySessionBucket'];

/**
 * Structural mirror of the server's `EntityHistoryResponse`. The
 * generated schema names it `EntityHistoryResponseSchema` (the
 * utoipa-derived wrapper). We re-state the runtime shape here so the
 * consumer stays clean of the `Schema` suffix and types `events` as
 * the real `EventEnvelope` rather than `unknown`.
 */
export interface EntityHistoryResponse {
  kind: string;
  id: string;
  display_name: string;
  events: EventEnvelopeFromGen[];
  next_after: string | null;
  session_breakdown: EntitySessionBucket[];
}

export async function getUserEntities(
  bearer: string,
  handle: string,
  opts: { after?: string; limit?: number } = {},
): Promise<EntitiesListResponse> {
  const qs = new URLSearchParams();
  if (opts.after !== undefined && opts.after !== '') qs.set('after', opts.after);
  if (opts.limit !== undefined) qs.set('limit', String(opts.limit));
  const suffix = qs.toString() ? `?${qs.toString()}` : '';
  return request<EntitiesListResponse>(
    'GET',
    `/v1/users/${encodeURIComponent(handle)}/entities${suffix}`,
    undefined,
    bearer,
  );
}

export async function getEntityHistory(
  bearer: string,
  handle: string,
  kind: string,
  id: string,
  opts: { after?: string; limit?: number } = {},
): Promise<EntityHistoryResponse> {
  const qs = new URLSearchParams();
  if (opts.after !== undefined && opts.after !== '') qs.set('after', opts.after);
  if (opts.limit !== undefined) qs.set('limit', String(opts.limit));
  const suffix = qs.toString() ? `?${qs.toString()}` : '';
  return request<EntityHistoryResponse>(
    'GET',
    `/v1/users/${encodeURIComponent(handle)}/entities/${encodeURIComponent(kind)}/${encodeURIComponent(id)}${suffix}`,
    undefined,
    bearer,
  );
}

// -- Organizations + org-share -------------------------------------
//
// Server endpoints live in `crates/starstats-server/src/org_routes.rs`
// and the org-share half of `sharing_routes.rs`. The slug is
// generated server-side; clients only ever pass a display name on
// create.

export type OrgDto = apiSchema['schemas']['OrgDto'];
export type OrgMemberDto = apiSchema['schemas']['OrgMemberDto'];
export type CreateOrgRequest = apiSchema['schemas']['CreateOrgRequest'];
export type CreateOrgResponse = apiSchema['schemas']['CreateOrgResponse'];
export type ListOrgsResponse = apiSchema['schemas']['ListOrgsResponse'];
export type GetOrgResponse = apiSchema['schemas']['GetOrgResponse'];
export type DeleteOrgResponse = apiSchema['schemas']['DeleteOrgResponse'];
export type AddMemberRequest = apiSchema['schemas']['AddMemberRequest'];
export type AddMemberResponse = apiSchema['schemas']['AddMemberResponse'];
export type RemoveMemberResponse =
  apiSchema['schemas']['RemoveMemberResponse'];
export type OrgShareEntry = apiSchema['schemas']['OrgShareEntry'];
export type ShareOrgRequest = apiSchema['schemas']['ShareOrgRequest'];
export type ShareOrgResponse = apiSchema['schemas']['ShareOrgResponse'];
export type RevokeOrgShareResponse =
  apiSchema['schemas']['RevokeOrgShareResponse'];

export async function createOrg(
  bearer: string,
  body: { name: string },
): Promise<CreateOrgResponse> {
  return postJson<CreateOrgResponse>(
    '/v1/orgs',
    { name: body.name } satisfies CreateOrgRequest,
    bearer,
  );
}

export async function listOrgs(bearer: string): Promise<ListOrgsResponse> {
  return request<ListOrgsResponse>('GET', '/v1/orgs', undefined, bearer);
}

export async function getOrg(
  bearer: string,
  slug: string,
): Promise<GetOrgResponse> {
  return request<GetOrgResponse>(
    'GET',
    `/v1/orgs/${encodeURIComponent(slug)}`,
    undefined,
    bearer,
  );
}

export async function deleteOrg(
  bearer: string,
  slug: string,
): Promise<DeleteOrgResponse> {
  return request<DeleteOrgResponse>(
    'DELETE',
    `/v1/orgs/${encodeURIComponent(slug)}`,
    undefined,
    bearer,
  );
}

export async function addOrgMember(
  bearer: string,
  slug: string,
  body: { handle: string; role: 'admin' | 'member' },
): Promise<AddMemberResponse> {
  return postJson<AddMemberResponse>(
    `/v1/orgs/${encodeURIComponent(slug)}/members`,
    { handle: body.handle, role: body.role } satisfies AddMemberRequest,
    bearer,
  );
}

export async function removeOrgMember(
  bearer: string,
  slug: string,
  handle: string,
): Promise<RemoveMemberResponse> {
  return request<RemoveMemberResponse>(
    'DELETE',
    `/v1/orgs/${encodeURIComponent(slug)}/members/${encodeURIComponent(handle)}`,
    undefined,
    bearer,
  );
}

export async function shareWithOrg(
  bearer: string,
  slug: string,
): Promise<ShareOrgResponse> {
  return postJson<ShareOrgResponse>(
    '/v1/me/share/org',
    { org_slug: slug } satisfies ShareOrgRequest,
    bearer,
  );
}

export async function unshareWithOrg(
  bearer: string,
  slug: string,
): Promise<RevokeOrgShareResponse> {
  return request<RevokeOrgShareResponse>(
    'DELETE',
    `/v1/me/share/org/${encodeURIComponent(slug)}`,
    undefined,
    bearer,
  );
}

// -- User preferences ----------------------------------------------
//
// Aliased to the generated schema's `UserPreferencesSchema`. Drift
// fix #5: the codegen has had this type for a while; lib/api.ts was
// just lagging behind its own TODO.
export type UserPreferences = apiSchema['schemas']['UserPreferencesSchema'];

export async function getPreferences(
  bearer: string,
): Promise<UserPreferences> {
  return request<UserPreferences>(
    'GET',
    '/v1/me/preferences',
    undefined,
    bearer,
  );
}

export async function putPreferences(
  bearer: string,
  prefs: UserPreferences,
): Promise<void> {
  await putJson<void>('/v1/me/preferences', prefs, bearer);
}

// -- Admin: SMTP config ---------------------------------------------

/** Fetch current SMTP config (password redacted; presence flag is on
 *  the response as `password_set`). 403 if caller isn't an admin. */
export async function getSmtpConfig(
  bearer: string,
): Promise<SmtpConfigResponse> {
  return request<SmtpConfigResponse>('GET', '/v1/admin/smtp', undefined, bearer);
}

/** Persist a new SMTP config + hot-swap the runtime mailer. The
 *  `password` field on the body tri-state: omit (null) = keep
 *  existing, "" = clear auth, non-empty = set new. Returns the
 *  re-read row so the form can refresh state from the server. */
export async function putSmtpConfig(
  body: SmtpConfigRequest,
  bearer: string,
): Promise<SmtpConfigResponse> {
  return putJson<SmtpConfigResponse>('/v1/admin/smtp', body, bearer);
}

/** Trigger a diagnostic email to the calling admin's verified
 *  address. 400 if email is unverified, 502 if the SMTP send fails. */
export async function testSmtp(
  bearer: string,
  toAddress?: string | null,
): Promise<TestSendResponse> {
  const body =
    toAddress && toAddress.trim().length > 0
      ? { to_address: toAddress.trim() }
      : {};
  return postJson<TestSendResponse>('/v1/admin/smtp/test', body, bearer);
}

// -- Admin: Ship Matrix config --------------------------------------

export type ShipMatrixConfigResponse =
  apiSchema['schemas']['ShipMatrixConfigResponse'];
export type ShipMatrixConfigRequest =
  apiSchema['schemas']['ShipMatrixConfigRequest'];

/** Fetch the Ship Matrix runtime config (the `media_enabled`
 *  kill-switch). 403 if caller isn't an admin. */
export async function getShipMatrixConfig(
  bearer: string,
): Promise<ShipMatrixConfigResponse> {
  return request<ShipMatrixConfigResponse>(
    'GET',
    '/v1/admin/ship-matrix',
    undefined,
    bearer,
  );
}

/** Persist + hot-swap the Ship Matrix media kill-switch. Effective
 *  immediately on the server (no redeploy). Returns the new state. */
export async function putShipMatrixConfig(
  body: ShipMatrixConfigRequest,
  bearer: string,
): Promise<ShipMatrixConfigResponse> {
  return putJson<ShipMatrixConfigResponse>(
    '/v1/admin/ship-matrix',
    body,
    bearer,
  );
}

// -- Discover (Piece 3 of public-profile UX) -----------------------
//
// `GET /v1/discover/profiles` returns the browsable listing of
// public profiles. Unauthenticated — the same data is reachable
// per-handle at `/v1/public/{handle}/*` so consolidating it into
// an index changes nothing about the trust posture.

export type DiscoverProfile = apiSchema['schemas']['DiscoverProfile'];
export type DiscoverProfilesResponse =
  apiSchema['schemas']['DiscoverProfilesResponse'];

export async function getDiscoverProfiles(
  options: { after?: string; limit?: number } = {},
): Promise<DiscoverProfilesResponse> {
  const params = new URLSearchParams();
  if (options.after !== undefined && options.after !== '') {
    params.set('after', options.after);
  }
  if (options.limit !== undefined) {
    params.set('limit', String(options.limit));
  }
  const qs = params.toString();
  const path =
    qs === '' ? '/v1/discover/profiles' : `/v1/discover/profiles?${qs}`;
  // No bearer — endpoint is intentionally public.
  return request<DiscoverProfilesResponse>('GET', path, undefined, undefined);
}

// -- Profile layout ------------------------------------------------
//
// Owner-only GET + PUT for the widget layout stored in `users.profile_layout`.
// NULL from GET means "owner hasn't customised yet; fall back to
// DEFAULT_LAYOUT on the web layer." The server does not know about
// the widget registry — unknown ids are filtered at render time.

export type LayoutEntry = apiSchema['schemas']['LayoutEntry'];
export type WidgetSize = apiSchema['schemas']['WidgetSize'];
export type LayoutSurface = apiSchema['schemas']['LayoutSurface'];
export type ProfileLayoutResponse = apiSchema['schemas']['ProfileLayoutResponse'];

export async function getProfileLayout(
  token: string,
  surface: LayoutSurface = 'profile',
): Promise<ProfileLayoutResponse> {
  return request<ProfileLayoutResponse>(
    'GET',
    `/v1/users/me/profile-layout?surface=${surface}`,
    undefined,
    token,
  );
}

export async function updateProfileLayout(
  token: string,
  layout: LayoutEntry[] | null,
  surface: LayoutSurface = 'profile',
): Promise<ProfileLayoutResponse> {
  return request<ProfileLayoutResponse>(
    'PUT',
    `/v1/users/me/profile-layout?surface=${surface}`,
    { layout },
    token,
  );
}

// -- Share scopes (Plan 3b Option A) --------------------------------
//
// Per-widget visitor visibility toggles stored in `users.share_scopes`
// JSONB. Owners opt in; all fields default to false (private).
//
// Three endpoints:
//   GET /v1/users/me/share-scopes       — owner reads own toggles
//   PUT /v1/users/me/share-scopes       — owner writes own toggles
//   GET /v1/public/:handle/share-scopes — visitor reads owner's toggles
//                                         (SpiceDB public-visibility gated)
//
// `WidgetShareScopes` is now in the generated schema after regeneration.
// Alias it here so existing call sites keep the stable `WidgetShareScopesApi` name.
//
// The generated schema marks all fields optional (reflecting `#[serde(default)]`
// on the Rust side). We `Required<>` them here because the server always returns
// all five fields, and `isAvailable` checks must return `boolean`, not
// `boolean | undefined`.
export type WidgetShareScopesApi = Required<apiSchema['schemas']['WidgetShareScopes']>;

/** Owner reads their own per-widget sharing toggles. */
export async function getMyShareScopes(
  token: string,
): Promise<WidgetShareScopesApi> {
  return request<WidgetShareScopesApi>(
    'GET',
    '/v1/users/me/share-scopes',
    undefined,
    token,
  );
}

/** Visitor reads the owner's per-widget sharing toggles.
 *  Gated by SpiceDB public-visibility; returns 404 when profile is
 *  not public. Falls back to all-false on any error — callers should
 *  use try/catch and default to all-false. */
export async function getPublicShareScopes(
  handle: string,
): Promise<WidgetShareScopesApi> {
  return request<WidgetShareScopesApi>(
    'GET',
    `/v1/public/${encodeURIComponent(handle)}/share-scopes`,
    undefined,
    undefined,
  );
}

/** Owner writes new per-widget sharing toggles. Returns the canonical
 *  form as stored by the server. */
export async function updateMyShareScopes(
  token: string,
  scopes: WidgetShareScopesApi,
): Promise<WidgetShareScopesApi> {
  return request<WidgetShareScopesApi>(
    'PUT',
    '/v1/users/me/share-scopes',
    scopes,
    token,
  );
}

// ---------------------------------------------------------------------------
// Plan 3b Option B — per-recipient ShareScope
// ---------------------------------------------------------------------------
//
// `ShareScope` is the JSONB clamp stored on `share_metadata.scope` for a
// (owner, recipient) pair. The type alias lives at the top of this file
// (~L1566) since it was already in use for share management. This block
// adds the visitor-side fetch wrapper.
//
// Returns `null` when no clamp is set (visitor has the relationship but
// the owner hasn't tightened/widened anything for them) — that's the
// pass-through case.

/** Visitor fetches their own per-recipient ShareScope on this profile.
 *  Returns `null` when no clamp is set. 404 means the visitor has no
 *  view permission (not shared with you, or owner doesn't exist).
 *
 *  Fail-open posture: callers should catch errors and treat them as
 *  `null` (no clamp). A missing scope is equivalent to "no Option B
 *  override applies" — Option A (per-owner share_scopes) still gates
 *  visibility. */
export async function getFriendScope(
  token: string,
  handle: string,
): Promise<ShareScope | null> {
  return request<ShareScope | null>(
    'GET',
    `/v1/u/${encodeURIComponent(handle)}/scope`,
    undefined,
    token,
  );
}

// -- Reference resolution -------------------------------------------
//
// -- Rich reference resolution for the paperdoll page --------------
//
// Returns the full KB entry for each class name, including slug,
// category, classification, and image availability. Used by
// /me/loadout to render ItemTile with image, link, and name.

export interface ResolvedItem {
  display_name: string;
  slug: string | null;
  category: string;
  classification: string | null;
  classification_label: string | null;
  has_image: boolean;
}

/**
 * Resolve an array of entity class names to their full KB reference
 * entries in one round-trip (POST /v1/reference/resolve).
 *
 * Returns a partial map — unknown class names are absent. Short-circuits
 * to `{}` on empty input so callers never guard against zero-length arrays.
 */
export async function resolveReferenceItems(
  bearer: string,
  classes: string[],
): Promise<Record<string, ResolvedItem>> {
  if (classes.length === 0) return {};
  const resp = await request<{ resolved: Record<string, ResolvedItem> }>(
    'POST',
    '/v1/reference/resolve',
    { class_names: classes },
    bearer,
  );
  return resp.resolved ?? {};
}

// ---------------------------------------------------------------------------
// Public-beta waitlist
// ---------------------------------------------------------------------------

export type WaitlistJoinRequest = apiSchema['schemas']['WaitlistJoinRequest'];
export type WaitlistJoinResponse = apiSchema['schemas']['WaitlistJoinResponse'];
export type WaitlistEntryApi = apiSchema['schemas']['WaitlistEntryApi'];
export type WaitlistConfigApi = apiSchema['schemas']['WaitlistConfigApi'];
export type WaitlistStatusResponse = apiSchema['schemas']['WaitlistStatusResponse'];

/**
 * Join the public-beta waitlist. Unauthenticated by necessity — the
 * caller does not have an account yet, that being the point.
 *
 * `position` is null when the server admitted them immediately (under the
 * cap) and a 1-based queue position otherwise.
 */
export async function joinWaitlist(
  input: WaitlistJoinRequest,
): Promise<WaitlistJoinResponse> {
  return postJson<WaitlistJoinResponse>('/v1/waitlist', input);
}

/**
 * Read the public beta gate state. Unauthenticated. Drives whether the
 * logged-out surface shows the waitlist overlay — the same flag
 * `/v1/auth/signup` enforces, so the overlay and the gate can't drift.
 */
export async function getWaitlistStatus(): Promise<WaitlistStatusResponse> {
  return request<WaitlistStatusResponse>('GET', '/v1/waitlist/status', undefined, undefined);
}

export async function getAdminWaitlist(
  token: string,
  params: { status?: 'queued' | 'admitted'; limit?: number } = {},
): Promise<WaitlistEntryApi[]> {
  const q = new URLSearchParams();
  if (params.status) q.set('status', params.status);
  if (params.limit) q.set('limit', String(params.limit));
  const qs = q.toString();
  return request<WaitlistEntryApi[]>(
    'GET',
    `/v1/admin/waitlist${qs ? `?${qs}` : ''}`,
    undefined,
    token,
  );
}

export async function admitWaitlist(
  token: string,
  ids: string[],
): Promise<{ admitted: number }> {
  return postJson<{ admitted: number }>(
    '/v1/admin/waitlist/admit',
    { ids },
    token,
  );
}

/**
 * Re-send invites to already-admitted rows using their EXISTING tokens —
 * no re-mint, so links already in inboxes stay valid. Recovers the case
 * where an auto-admit minted an invite but the mail never sent (e.g. the
 * SMTP transport was down). `resent` counts successful sends, so it drops
 * below `ids.length` when the transport is still failing.
 */
export async function resendWaitlist(
  token: string,
  ids: string[],
): Promise<{ resent: number }> {
  return postJson<{ resent: number }>(
    '/v1/admin/waitlist/resend',
    { ids },
    token,
  );
}

export async function deleteWaitlist(
  token: string,
  ids: string[],
): Promise<{ deleted: string[]; blocked: string[] }> {
  return postJson<{ deleted: string[]; blocked: string[] }>(
    '/v1/admin/waitlist/delete',
    { ids },
    token,
  );
}

export async function getWaitlistConfig(
  token: string,
): Promise<WaitlistConfigApi> {
  return request<WaitlistConfigApi>(
    'GET',
    '/v1/admin/waitlist/config',
    undefined,
    token,
  );
}

export async function setWaitlistConfig(
  token: string,
  cfg: WaitlistConfigApi,
): Promise<WaitlistConfigApi> {
  return request<WaitlistConfigApi>(
    'PUT',
    '/v1/admin/waitlist/config',
    cfg,
    token,
  );
}

// ---------------------------------------------------------------------------
// Sitewide appearance defaults
// ---------------------------------------------------------------------------

export type AppearanceConfigApi = apiSchema['schemas']['AppearanceConfigApi'];

/**
 * Read the sitewide appearance defaults (today: the theme-switch wave
 * speed). Unauthenticated — the root layout needs this before any auth
 * exists so it can stamp `<html data-wave-speed>` for signed-out
 * visitors too. Mirrors `getWaitlistStatus`.
 */
export async function getAppearanceConfig(): Promise<AppearanceConfigApi> {
  return request<AppearanceConfigApi>(
    'GET',
    '/v1/appearance',
    undefined,
    undefined,
  );
}

/** Admin read of the same config (identical shape; separate route so the
 *  public GET stays unauthenticated while the admin console can still be
 *  gated). */
export async function getAdminAppearance(
  token: string,
): Promise<AppearanceConfigApi> {
  return request<AppearanceConfigApi>(
    'GET',
    '/v1/admin/appearance',
    undefined,
    token,
  );
}

export async function setAdminAppearance(
  token: string,
  cfg: AppearanceConfigApi,
): Promise<AppearanceConfigApi> {
  return request<AppearanceConfigApi>(
    'PUT',
    '/v1/admin/appearance',
    cfg,
    token,
  );
}
