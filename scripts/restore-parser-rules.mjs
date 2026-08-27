#!/usr/bin/env node
// Re-publish the recovered parser rules.
//
// WHY THIS EXISTS. The served rule set lives only as rows in `parser_rules`,
// written by a moderator publishing through `POST /v1/admin/parser-rules`.
// There is no seed and no export, so when the table was emptied the only
// surviving copy was one tray's local manifest cache. That copy is now
// committed at docs/recovery/parser-rules-2026-07-21.json, and this script
// puts it back.
//
// The admin UI cannot do this: /admin/parser-rules lists existing rules and
// toggles them by re-POSTing their own fields, so with an empty table it
// shows nothing and offers no way to create one.
//
// Usage:
//   STARSTATS_ADMIN_TOKEN=<moderator bearer> node scripts/restore-parser-rules.mjs [--dry-run]
//
// The token is the `t` field of the `starstats_session` cookie on the site,
// for an account with a moderator role. It is never printed by this script.
//
// ORDERING. Restore these BEFORE fixing the parser-manifest signing key.
// Trays currently reject the live manifest (the server's key no longer
// matches their pinned pubkey) and fall back to last-known-good — which is
// the only reason they still hold these rules. Fix the key against an empty
// table and every tray will verify and adopt an empty manifest, dropping what
// it was running on.

import { readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { dirname, join } from 'node:path';

const here = dirname(fileURLToPath(import.meta.url));
const RECOVERY = join(here, '..', 'docs', 'recovery', 'parser-rules-2026-07-21.json');
const API = (process.env.STARSTATS_API_URL ?? 'https://api.starstats.app').replace(/\/+$/, '');
const TOKEN = process.env.STARSTATS_ADMIN_TOKEN;
const DRY = process.argv.includes('--dry-run');

function die(msg) {
  console.error(`error: ${msg}`);
  process.exit(1);
}

const doc = JSON.parse(readFileSync(RECOVERY, 'utf8'));
const rules = doc.rules ?? [];
if (rules.length === 0) die(`no rules in ${RECOVERY}`);

console.log(`${rules.length} rule(s) from ${doc.fetched_at} → ${API}`);
for (const r of rules) {
  console.log(`  ${r.id.padEnd(24)} ${r.event_name}`);
}

if (DRY) {
  console.log('[dry-run] nothing sent');
  process.exit(0);
}
if (!TOKEN) die('STARSTATS_ADMIN_TOKEN not set (see the header of this file)');

let failed = 0;
for (const r of rules) {
  // Field names mirror PublishRuleRequest in admin_parser_rules.rs. The
  // upsert is keyed on rule_id, so re-running this is safe.
  const body = JSON.stringify({
    rule_id: r.id,
    event_name: r.event_name,
    match_kind: r.match_kind ?? 'event_name',
    body_regex: r.body_regex ?? '',
    fields: r.fields ?? [],
    enabled: true,
  });
  let resp;
  try {
    resp = await fetch(`${API}/v1/admin/parser-rules`, {
      method: 'POST',
      headers: {
        'content-type': 'application/json',
        authorization: `Bearer ${TOKEN}`,
      },
      body,
    });
  } catch (e) {
    console.error(`  FAIL ${r.id}: ${e.message}`);
    failed++;
    continue;
  }
  if (!resp.ok) {
    // Never echo the token; the status and the server's reason are enough.
    const text = await resp.text().catch(() => '');
    console.error(`  FAIL ${r.id}: ${resp.status} ${text.slice(0, 160)}`);
    if (resp.status === 401 || resp.status === 403) {
      die('token rejected — it must belong to an account with a moderator role');
    }
    failed++;
    continue;
  }
  console.log(`  ok   ${r.id}`);
}

// Verify against the endpoint clients actually read, not the one just written.
try {
  const check = await fetch(`${API}/v1/parser-definitions`);
  const served = await check.json();
  const n = (served.rules ?? []).length;
  console.log(`served manifest now reports ${n} rule(s)`);
  if (n < rules.length) {
    console.error(
      `warning: expected at least ${rules.length}; a rule may have published disabled`,
    );
  }
} catch (e) {
  console.error(`could not verify the served manifest: ${e.message}`);
}

process.exit(failed > 0 ? 1 : 0);
