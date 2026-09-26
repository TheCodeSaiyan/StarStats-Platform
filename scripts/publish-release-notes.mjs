#!/usr/bin/env node
// Send a release's notes to the server, where the tray's What's New and the
// web's /whats-new and /changelog read them.
//
//   node scripts/publish-release-notes.mjs --tag tray-v0.1.31
//
// Env (the same secrets the roadmap emit step already has, so CI needs no
// new credential):
//   ROADMAP_CI_EVENT_HMAC_KEY  signing key
//   ROADMAP_EVENTS_URL         .../v1/internal/roadmap/events; the release
//                              endpoint is derived from its origin
//   RELEASE_NOTES_URL          optional explicit override
//   GH_TOKEN                   for PR and roadmap lookups
//
// NEVER fails the release. Every problem, including missing secrets and a
// server that does not know the route yet, exits 0 with a warning: notes
// are not a reason for a release not to ship. Retries 5xx and network
// errors three times.

import crypto from 'node:crypto';
import { generateNotes } from './lib/release-notes-gen.mjs';

const warn = (msg) => {
  console.log(`::warning::[release-notes] ${msg}`);
  process.exit(0);
};

const args = process.argv.slice(2);
const tag = args[args.indexOf('--tag') + 1];
if (args.indexOf('--tag') === -1 || !tag) warn('usage: publish-release-notes.mjs --tag <tag>');

const key = process.env.ROADMAP_CI_EVENT_HMAC_KEY;
let url = process.env.RELEASE_NOTES_URL;
if (!url && process.env.ROADMAP_EVENTS_URL) {
  try {
    url = `${new URL(process.env.ROADMAP_EVENTS_URL).origin}/v1/internal/releases`;
  } catch {
    // fall through to the missing-url warning
  }
}
if (!key || !url) warn('signing key or endpoint not configured; notes not sent');

let notes;
try {
  // --offline skips PR and roadmap lookups; used by the tests.
  notes = await generateNotes({ tag, offline: args.includes('--offline') });
} catch (e) {
  warn(`could not build notes for ${tag}: ${e.message}`);
}

const body = JSON.stringify({
  schema_version: 1,
  track: notes.track,
  tag: notes.tag,
  version: notes.version,
  channel: notes.channel,
  date: notes.date,
  summary: notes.summary,
  groups: notes.groups,
});
const ts = Date.now().toString();
const sig = `v1=${crypto.createHmac('sha256', key).update(`v1.${ts}.`).update(body).digest('hex')}`;

const delays = [0, 1000, 4000];
for (let i = 0; i < delays.length; i++) {
  if (delays[i]) await new Promise((r) => setTimeout(r, delays[i]));
  try {
    const res = await fetch(url, {
      method: 'POST',
      headers: {
        'content-type': 'application/json',
        'X-StarStats-Timestamp': ts,
        'X-StarStats-Signature': sig,
      },
      body,
    });
    if (res.ok) {
      console.log(`[release-notes] sent ${tag}: ${notes.summary || 'nothing player-facing'}`);
      process.exit(0);
    }
    if (res.status < 500) warn(`server refused ${tag}: ${res.status} ${await res.text()}`);
  } catch (e) {
    if (i === delays.length - 1) warn(`could not reach the server: ${e.message}`);
  }
}
warn(`server kept failing for ${tag}; notes not sent`);
