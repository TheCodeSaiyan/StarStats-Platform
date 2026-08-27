#!/usr/bin/env node
// Rotate or verify the parser-manifest signing key.
//
//   node scripts/parser-signing-key.mjs --check      # compare stored vs pinned
//   node scripts/parser-signing-key.mjs --rotate     # generate + store a new key
//   node scripts/parser-signing-key.mjs --rotate --dry-run
//
// Both modes need `op signin` first.
//
// WHY THIS EXISTS. The server signs `/v1/parser-definitions` with an ed25519
// seed mounted from 1Password; the tray pins the PUBLIC half as a build-time
// constant. If the two drift apart, every client rejects every manifest and
// silently runs on last-known-good rules — which is what happened between
// 2026-07-21 and 2026-08-27 and took a full investigation to spot, because
// nothing about it is visible from outside: the endpoint keeps returning a
// perfectly healthy signed 200.
//
// THE SEED IS NEVER PRINTED, never written to disk, and never passed as an
// argument (argv is visible in the process list). It is piped to `op` as a
// JSON template on stdin. 1Password keeps item history, so a rotation is
// reversible.
//
// AFTER ROTATING, BOTH HALVES MUST MOVE:
//   1. pin the printed public key in crates/starstats-client/src/parser_defs.rs
//   2. RECREATE the API container — `parser_signing_key()` caches the key in a
//      OnceLock, so a running process never re-reads the mounted file. A
//      secret re-render alone changes nothing; this is the step that is easy
//      to miss and looks exactly like a failed rotation.
//   3. verify with --check, and by fetching the manifest and checking its
//      signature against the new pin.
//
// ORDERING, learned the hard way: make sure the served rule set is NOT empty
// before restoring verification. A client that can verify will adopt whatever
// it is given, including an empty manifest, and drop the rules it was running
// on. See docs/PARSER_DEFINITION_UPDATES.md.

import { execFileSync } from 'node:child_process';
import { createPrivateKey, createPublicKey, generateKeyPairSync } from 'node:crypto';
import { readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { dirname, join } from 'node:path';

const ITEM = '5salo5qnqvzbfxcqzhb4cniuf4';
const VAULT = 'Homelab';
const FIELD = 'credential';
const REF = `op://${VAULT}/${ITEM}/${FIELD}`;
// The 16-byte PKCS#8 prefix for a raw ed25519 private key.
const PKCS8_PREFIX = Buffer.from('302e020100300506032b657004220420', 'hex');

const args = process.argv.slice(2);
const CHECK = args.includes('--check');
const ROTATE = args.includes('--rotate');
const DRY = args.includes('--dry-run');

function die(msg) {
  console.error(`error: ${msg}`);
  process.exit(1);
}

/** The public half of a raw 32-byte ed25519 seed, base64. */
function publicFromSeed(seed) {
  if (seed.length !== 32) die(`seed decodes to ${seed.length} bytes, expected 32`);
  const key = createPrivateKey({
    key: Buffer.concat([PKCS8_PREFIX, seed]),
    format: 'der',
    type: 'pkcs8',
  });
  return createPublicKey(key).export({ format: 'der', type: 'spki' }).subarray(-32).toString('base64');
}

/** The key the tray is built to trust. Read from source so the two cannot drift. */
function pinnedKey() {
  const here = dirname(fileURLToPath(import.meta.url));
  const src = readFileSync(
    join(here, '..', 'crates', 'starstats-client', 'src', 'parser_defs.rs'),
    'utf8',
  );
  const m = src.match(/PARSER_SIGNING_PUBKEY_B64[^=]*=\s*\n?\s*Some\("([^"]+)"\)/);
  return m ? m[1] : null;
}

if (CHECK === ROTATE) {
  die('pass exactly one of --check or --rotate');
}

const pinned = pinnedKey();
if (!pinned) die('could not read the pinned key from parser_defs.rs');

if (CHECK) {
  let stored;
  try {
    stored = execFileSync('op', ['read', REF], { encoding: 'utf8' }).trim();
  } catch (e) {
    die(`could not read the 1Password item — signed in? (${e.message})`);
  }
  const pub = publicFromSeed(Buffer.from(stored, 'base64'));
  console.log(`public key of the stored seed: ${pub}`);
  console.log(`pinned in parser_defs.rs:      ${pinned}`);
  if (pub === pinned) {
    console.log('MATCH — 1Password holds the key the tray trusts.');
    console.log('If the served manifest still does not verify, the API has not');
    console.log('been RECREATED: the key is cached in a OnceLock for the life of');
    console.log('the process, so a secret re-render alone changes nothing.');
    process.exit(0);
  }
  console.log('MISMATCH — the stored seed is not the pinned key.');
  process.exit(1);
}

// --rotate
const { privateKey } = generateKeyPairSync('ed25519');
const seed = privateKey.export({ format: 'der', type: 'pkcs8' }).subarray(-32);
const pub = publicFromSeed(seed);
console.log(`new public key (pin this in parser_defs.rs):\n  ${pub}`);
if (DRY) {
  console.log('[dry-run] 1Password not touched');
  process.exit(0);
}

let current;
try {
  current = JSON.parse(
    execFileSync('op', ['item', 'get', ITEM, '--vault', VAULT, '--format', 'json', '--reveal'], {
      encoding: 'utf8',
    }),
  );
} catch (e) {
  die(`could not read the 1Password item — signed in? (${e.message})`);
}
const field = (current.fields ?? []).find((f) => f.id === FIELD || f.label === FIELD);
if (!field) {
  die(`no '${FIELD}' field on that item; found: ${(current.fields ?? []).map((f) => f.id).join(', ')}`);
}
field.value = seed.toString('base64');

try {
  execFileSync('op', ['item', 'edit', ITEM, '--vault', VAULT, '--format', 'json'], {
    input: JSON.stringify(current),
    encoding: 'utf8',
    stdio: ['pipe', 'ignore', 'inherit'],
  });
} catch (e) {
  die(`1Password update failed (${e.message})`);
}

console.log('1Password updated. Now: pin the key above, RECREATE starstats-api');
console.log('(not just re-render the secret), then run --check.');
