#!/usr/bin/env node
// Emit a signed CI event to the roadmap pipeline server.
//
// Spec: docs/ROADMAP-PIPELINE-SPEC.md §4. Sends one POST to the
// `/v1/internal/roadmap/events` endpoint, signed with an HMAC-SHA256
// MAC over `v1.<timestamp_ms>.<body>` (mirrors the Revolut webhook
// scheme; matches the server's `verify_event_signature` helper).
//
// Required env vars:
//   - ROADMAP_CI_EVENT_HMAC_KEY  Shared secret (raw bytes/UTF-8).
//   - ROADMAP_EVENTS_URL         Full URL of the events endpoint, e.g.
//                                `https://api.starstats.app/v1/internal/roadmap/events`.
//   - ROADMAP_ITEM_SLUG          Slug of the roadmap item this release
//                                ships. Without it the script no-ops —
//                                a release that doesn't correspond to a
//                                tracked roadmap item simply has nothing
//                                to emit. Multi-item releases call this
//                                script in a loop, once per slug.
//   - CHANNEL                    `alpha` | `beta` | `rc` | `live` (from
//                                the `validate-tag` job in release.yml).
//   - COMMIT_SHA                 Tag commit SHA.
//   - BUILD_ID                   `github.run_id`.
//   - CI_RUN_URL                 Link to the Actions run.
//
// Optional:
//   - TAG                        The git tag (`vX.Y.Z[-pre.N]`).
//
// Exit codes:
//   0 — sent, OR no-op (missing config / no slug to emit for / 404
//       from the receiver, which indicates the roadmap item slug
//       isn't seeded yet / 400 with an unknown-channel hint, which
//       indicates the server is lagging a client-side CHANNEL_MAP
//       expansion — both are config / rollout-order gaps, not
//       release-blocking).
//   1 — non-retryable failure (4xx OTHER THAN 404 / "unknown channel"
//       400s — auth, schema, bad payload — or exhausted retries on 5xx).
//   2 — config error (missing required env when emission was intended).

import crypto from 'node:crypto';

function noop(msg) {
  console.log(`[roadmap-emit] no-op: ${msg}`);
  process.exit(0);
}

function fatal(code, msg) {
  console.error(`[roadmap-emit] ${msg}`);
  process.exit(code);
}

const env = process.env;

// Soft-skip when the pipeline isn't wired yet (Phase 0 prerequisites
// not met). This lets the workflow step ship today without needing
// repo secrets configured — it just no-ops until they exist.
if (!env.ROADMAP_CI_EVENT_HMAC_KEY) {
  noop('ROADMAP_CI_EVENT_HMAC_KEY not set (pipeline not configured)');
}
if (!env.ROADMAP_EVENTS_URL) {
  noop('ROADMAP_EVENTS_URL not set (pipeline not configured)');
}
if (!env.ROADMAP_ITEM_SLUG) {
  noop('ROADMAP_ITEM_SLUG not set (no item to emit for)');
}

for (const k of ['CHANNEL', 'COMMIT_SHA', 'BUILD_ID', 'CI_RUN_URL']) {
  if (!env[k]) fatal(2, `missing required env: ${k}`);
}

// Map the release channel onto the spec's ChannelName + new_status
// pair. The pipeline tracks `live | beta | rc | alpha | tech-preview`
// (spec §2.1). As of v1.8.9, `rc` is a first-class peer of beta on
// the release ladder (alpha → beta → rc → live); each CI tag-suffix
// routes to its own channel with new_status='shipped'. Previously rc
// folded into channel='beta', new_status='beta'; the spec change
// makes rc-track shipments independently observable.
const CHANNEL_MAP = {
  live:  { channel: 'live',  new_status: 'shipped' },
  beta:  { channel: 'beta',  new_status: 'shipped' },
  rc:    { channel: 'rc',    new_status: 'shipped' },
  alpha: { channel: 'alpha', new_status: 'shipped' },
};

const mapped = CHANNEL_MAP[env.CHANNEL];
if (!mapped) fatal(2, `unknown CHANNEL: ${env.CHANNEL}`);

const payload = {
  schema_version: 1,
  event_id: crypto.randomUUID(),
  project_item_id: null,
  roadmap_slug: env.ROADMAP_ITEM_SLUG,
  channel: mapped.channel,
  new_status: mapped.new_status,
  commit_sha: env.COMMIT_SHA,
  build_id: env.BUILD_ID,
  ci_run_url: env.CI_RUN_URL,
  tag: env.TAG || null,
  // The server re-reads `public` from GraphQL anyway (spec §4.3), so
  // optimistically claim true here; a mismatch surfaces in the audit
  // log without affecting the state transition.
  public: true,
  build_health: null,
  coverage_delta: null,
};

const body = JSON.stringify(payload);
const ts_ms = Date.now().toString();

const mac = crypto.createHmac('sha256', env.ROADMAP_CI_EVENT_HMAC_KEY);
mac.update(`v1.${ts_ms}.`);
mac.update(body);
const sig = `v1=${mac.digest('hex')}`;

const url = env.ROADMAP_EVENTS_URL;
const headers = {
  'content-type': 'application/json',
  'X-StarStats-Timestamp': ts_ms,
  'X-StarStats-Signature': sig,
};

// 3 attempts with 0/1s/4s backoff. Retries on 5xx + network errors;
// surfaces 4xx (including 401 signature failures) immediately.
const delays = [0, 1000, 4000];
let lastErr = null;
for (let i = 0; i < delays.length; i++) {
  if (delays[i] > 0) {
    await new Promise((r) => setTimeout(r, delays[i]));
  }
  try {
    const resp = await fetch(url, { method: 'POST', headers, body });
    if (resp.ok) {
      console.log(
        `[roadmap-emit] ok (attempt ${i + 1}): status=${resp.status} ` +
          `event_id=${payload.event_id} slug=${payload.roadmap_slug} ` +
          `channel=${payload.channel} new_status=${payload.new_status}`,
      );
      process.exit(0);
    }
    const text = await resp.text();
    if (resp.status >= 500) {
      console.warn(
        `[roadmap-emit] attempt ${i + 1} 5xx: ${resp.status} ${text.slice(0, 200)}`,
      );
      lastErr = `5xx ${resp.status}`;
      continue;
    }
    // 404 — the receiver doesn't know the slug. Treat as a soft
    // failure: the release itself is fine, the telemetry hop is
    // optional, and a missing roadmap item is a config gap (the
    // item needs seeding) not a code / auth bug. Don't block the
    // Release tray / Release images jobs on this. Surfaced on
    // v1.8.4-alpha.5 + v1.8.4-alpha.6 with slug=`smoke-test`.
    if (resp.status === 404) {
      console.warn(
        `[roadmap-emit] soft-fail 404 (slug=${payload.roadmap_slug} not seeded?): ${text.slice(0, 200)}`,
      );
      process.exit(0);
    }
    // 400 with an unknown-channel hint — same root cause as 404 but
    // from the other side: the client (this script's CHANNEL_MAP) is
    // ahead of the production server's accepted ChannelName enum.
    // Surfaced on tray-v1.8.9-rc.1 when PR #125 added `rc` as a
    // first-class channel; production server was still on v1.8.9
    // (pre-#125), so `channel: 'rc'` 400'd. Treat as soft failure:
    // a release should not be blocked by the rollout-order
    // chicken-and-egg between client + server. Once the server
    // catches up (v1.8.10+ on `:latest`), this branch stops firing
    // and telemetry resumes naturally.
    if (resp.status === 400 && /unknown.*channel/i.test(text)) {
      console.warn(
        `[roadmap-emit] soft-fail 400 unknown-channel (channel=${payload.channel} not recognized by server; server lagging client?): ${text.slice(0, 200)}`,
      );
      process.exit(0);
    }
    // Other 4xx — non-retryable (auth, bad payload, schema mismatch).
    fatal(1, `non-retryable ${resp.status}: ${text.slice(0, 500)}`);
  } catch (e) {
    console.warn(`[roadmap-emit] attempt ${i + 1} threw: ${e.message}`);
    lastErr = e.message;
  }
}
fatal(1, `exhausted retries; last error: ${lastErr}`);
