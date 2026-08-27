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
//   node scripts/restore-parser-rules.mjs --dry-run          # no token needed
//   node scripts/restore-parser-rules.mjs --token-file tok    # preferred
//   cat tok | node scripts/restore-parser-rules.mjs           # also fine
//
// A CREDENTIAL MUST NOT GO ON THE COMMAND LINE. Arguments and inline
// `VAR=... cmd` prefixes are recorded in shell history and are visible in the
// process list to anything else on the machine. This script therefore reads
// the token from a FILE or from STDIN. `STARSTATS_ADMIN_TOKEN` is still
// honoured for CI, where the value comes from a secret store rather than a
// keyboard, but it is the last resort and the script says so.
//
// The value can be EITHER a bare JWT or the whole `starstats_session` cookie
// as copied from devtools — that cookie is URL-encoded JSON like
// `%7B%22t%22%3A%22eyJ...`, and pasting it verbatim is the obvious thing to
// do, so the script decodes it and takes the `t` field rather than sending
// the blob and failing on a 401 that explains nothing.
//
// The account needs a moderator role. The token is never printed.
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
const DRY = process.argv.includes('--dry-run');

/**
 * Accept a bare JWT or a whole `starstats_session` cookie.
 *
 * The cookie is URL-encoded JSON (`%7B%22t%22%3A%22eyJ...`), which is what
 * devtools puts on the clipboard, so it is the value people actually have.
 * Sending it as a bearer token 401s with nothing useful.
 */
function extractToken(raw) {
  const value = String(raw ?? '').trim();
  if (!value) return null;
  if (value.startsWith('ey')) return value; // already a JWT
  let text = value;
  if (text.includes('%')) {
    try {
      text = decodeURIComponent(text);
    } catch {
      /* not encoded after all */
    }
  }
  if (text.startsWith('{')) {
    try {
      const t = JSON.parse(text)?.t;
      if (typeof t === 'string' && t.length > 0) return t;
    } catch {
      return null;
    }
  }
  return text;
}

function readToken() {
  const i = process.argv.indexOf('--token-file');
  if (i !== -1) {
    const path = process.argv[i + 1];
    if (!path) die('--token-file needs a path');
    return extractToken(readFileSync(path, 'utf8'));
  }
  // Piped stdin, if there is any.
  if (!process.stdin.isTTY) {
    try {
      const piped = readFileSync(0, 'utf8');
      if (piped.trim()) return extractToken(piped);
    } catch {
      /* no stdin */
    }
  }
  if (process.env.STARSTATS_ADMIN_TOKEN) {
    console.error(
      'note: reading the token from the environment. If you typed it inline ' +
        'on the command line it is now in your shell history — prefer ' +
        '--token-file, and clear the entry.',
    );
    return extractToken(process.env.STARSTATS_ADMIN_TOKEN);
  }
  return null;
}

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
const TOKEN = readToken();
if (!TOKEN) {
  die(
    'no token. Pass --token-file <path>, or pipe it in. It may be a bare JWT ' +
      'or the whole starstats_session cookie. See the header of this file.',
  );
}

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
