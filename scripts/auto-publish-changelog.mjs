#!/usr/bin/env node
// Auto-publish roadmap changelog drafts for one slug via the
// HMAC-keyed internal endpoint.
//
// Companion to `roadmap-emit-event.mjs`: same HMAC signing scheme
// (`v1.<timestamp_ms>.<body>`), same secret key (`ROADMAP_CI_EVENT_HMAC_KEY`),
// different endpoint. Replaces the JWT-based call to
// `/v1/admin/roadmap/changelog/{id}/publish` that the previous
// auto-publish CI job used — JWTs expire on ~1-hour cadences and
// forced a rotation chore for every release.
//
// Server-side endpoint: spec §8.5 / `internal_changelog_routes.rs`.
//
// Required env vars:
//   - ROADMAP_CI_EVENT_HMAC_KEY  Shared secret (raw bytes / UTF-8).
//                                Same key as the emit endpoint.
//   - STARSTATS_API_URL          Base URL of starstats-server, e.g.
//                                `https://api.starstats.app`. The publish
//                                endpoint path (`/v1/internal/roadmap/
//                                changelog/publish`) is appended
//                                automatically. Shared with the existing
//                                admin-CLI publish path; one secret, one
//                                source of truth for the API hostname.
//   - ROADMAP_ITEM_SLUG          Slug to publish drafts for. Required.
//
// Optional:
//   - CHANNEL                    Channel filter (alpha|beta|rc|live|tech-preview).
//                                When unset, all drafts for the slug are
//                                published. Pass `live` from the auto-publish
//                                CI job so historical un-published alpha/beta
//                                drafts don't accidentally publish on a live
//                                release.
//   - MAX_TO_PUBLISH             Per-call cap (server clamps to 50).
//
// Exit codes:
//   0 — published (one or more) OR no-op (zero matched / 404 slug-not-
//       seeded; both are config-class outcomes that should not block
//       releases, mirroring `roadmap-emit-event.mjs` semantics).
//   1 — non-retryable failure (auth, schema, server error after retries).
//   2 — config error (missing required env).

import crypto from 'node:crypto';

function noop(msg) {
  console.log(`[auto-publish] no-op: ${msg}`);
  process.exit(0);
}

function fatal(code, msg) {
  console.error(`[auto-publish] ${msg}`);
  process.exit(code);
}

const env = process.env;

// Soft-skip the entire job when the pipeline isn't wired yet — same
// disposition as `roadmap-emit-event.mjs`.
if (!env.ROADMAP_CI_EVENT_HMAC_KEY) {
  noop('ROADMAP_CI_EVENT_HMAC_KEY not set (pipeline not configured)');
}
if (!env.STARSTATS_API_URL) {
  noop('STARSTATS_API_URL not set (pipeline not configured)');
}
if (!env.ROADMAP_ITEM_SLUG) {
  noop('ROADMAP_ITEM_SLUG not set (no item to publish for)');
}

const payload = {
  schema_version: 1,
  event_id: crypto.randomUUID(),
  roadmap_slug: env.ROADMAP_ITEM_SLUG,
};
if (env.CHANNEL) {
  payload.channel = env.CHANNEL;
}
if (env.MAX_TO_PUBLISH) {
  const n = Number.parseInt(env.MAX_TO_PUBLISH, 10);
  if (Number.isInteger(n) && n > 0) {
    payload.max_to_publish = n;
  }
}

const body = JSON.stringify(payload);
const ts_ms = Date.now().toString();

// Sign exactly the bytes of `v1.<ts>.<body>` per spec §4.5 — same
// scheme as the emit endpoint so the server's verify_event_signature
// accepts the result.
const mac = crypto.createHmac('sha256', env.ROADMAP_CI_EVENT_HMAC_KEY);
mac.update(`v1.${ts_ms}.`);
mac.update(body);
const sig = `v1=${mac.digest('hex')}`;

// Compose the endpoint URL from the base. Trim trailing slashes so
// `STARSTATS_API_URL=https://api.starstats.app/` and the bare form
// both resolve identically.
const url = `${env.STARSTATS_API_URL.replace(/\/+$/, "")}/v1/internal/roadmap/changelog/publish`;
const headers = {
  'content-type': 'application/json',
  'X-StarStats-Timestamp': ts_ms,
  'X-StarStats-Signature': sig,
};

// 3 attempts with 0/1s/4s backoff. Retries on 5xx + network errors;
// surfaces 4xx immediately (auth = 401 fatal; missing slug = 404 soft;
// other 4xx = fatal). Mirrors the emit script's retry shape.
const delays = [0, 1000, 4000];
let lastErr = null;
for (let i = 0; i < delays.length; i++) {
  if (delays[i] > 0) {
    await new Promise((r) => setTimeout(r, delays[i]));
  }
  try {
    const resp = await fetch(url, { method: 'POST', headers, body });
    const text = await resp.text();
    if (resp.ok) {
      // 200 with JSON body: { published, skipped, entries[] }
      let parsed = null;
      try {
        parsed = JSON.parse(text);
      } catch {
        // Server should always return JSON on 200; treat as success
        // but flag the unexpected shape.
        console.warn(
          `[auto-publish] ok (attempt ${i + 1}) but body wasn't JSON: ${text.slice(0, 200)}`,
        );
        process.exit(0);
      }
      console.log(
        `[auto-publish] ok (attempt ${i + 1}): published=${parsed.published ?? '?'} ` +
          `skipped=${parsed.skipped ?? '?'} slug=${payload.roadmap_slug} ` +
          `channel=${payload.channel || 'all'}`,
      );
      if (Array.isArray(parsed.entries)) {
        for (const e of parsed.entries) {
          console.log(`  - ${e.id} [${e.channel}] ${e.title}`);
        }
      }
      process.exit(0);
    }
    if (resp.status >= 500) {
      console.warn(
        `[auto-publish] attempt ${i + 1} 5xx: ${resp.status} ${text.slice(0, 200)}`,
      );
      lastErr = `5xx ${resp.status}`;
      continue;
    }
    // 404 — slug not seeded server-side. Same soft-fail disposition
    // as `roadmap-emit-event.mjs`: a release should not be blocked by
    // an unseeded slug; the operator notices via the log line.
    if (resp.status === 404) {
      console.warn(
        `[auto-publish] soft-fail 404 (slug=${payload.roadmap_slug} not seeded?): ${text.slice(0, 200)}`,
      );
      process.exit(0);
    }
    // Other 4xx — non-retryable.
    fatal(1, `non-retryable ${resp.status}: ${text.slice(0, 500)}`);
  } catch (e) {
    console.warn(`[auto-publish] attempt ${i + 1} threw: ${e.message}`);
    lastErr = e.message;
  }
}
fatal(1, `exhausted retries; last error: ${lastErr}`);
