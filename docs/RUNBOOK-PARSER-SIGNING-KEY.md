# Runbook: the parser-manifest signing key

`GET /v1/parser-definitions` is signed with an ed25519 key. The two halves
live apart, and keeping them together is the whole job:

| half | where | changed by |
| --- | --- | --- |
| private seed (32 bytes, base64) | 1Password, mounted into `starstats-api` as `/run/secrets/starstats_parser_signing_key` | re-render + **container recreate** |
| public key | `PARSER_SIGNING_PUBKEY_B64` in `crates/starstats-client/src/parser_defs.rs` | a tray release |

## Why this needs a runbook

**The failure looks like health.** When the halves diverge, the endpoint keeps
returning a signed `200`. Nothing 500s, nothing alerts. Every tray rejects
every manifest and quietly runs on last-known-good rules; the only trace is a
`WARN parser manifest rejected by signature policy ... sig_valid=Some(false)`
line in the client log, buried among thousands.

That is not hypothetical. The halves diverged some time after 2026-07-21 and
were not noticed until 2026-08-27.

## Check

    op signin && node scripts/parser-signing-key.mjs --check

Derives the public half of the stored seed and compares it to the pin, which
it reads out of `parser_defs.rs` rather than being told, so the check cannot
drift from what is actually built. The seed is never printed.

To check the other end (what the server is really signing with), fetch the
manifest and verify its signature against the pinned key. `--check` passing
while the manifest still fails means the API has not been recreated.

## Rotate

    op signin && node scripts/parser-signing-key.mjs --rotate --dry-run
    op signin && node scripts/parser-signing-key.mjs --rotate

Then, and all three are required:

1. Pin the printed public key in `parser_defs.rs`.
2. **Recreate the API container.** `parser_signing_key()` caches the key in a
   `OnceLock`, so a running process reads the mounted file exactly once, at
   startup, and never again. Re-rendering the secret changes nothing on its
   own. This is the step that gets missed, and missing it is
   indistinguishable from a failed rotation.
3. Ship a tray release carrying the new pin.

The seed is never printed, never written to disk, and never passed as an
argument (argv is visible in the process list). It is piped to `op` as a JSON
template on stdin. 1Password keeps item history, so a rotation is reversible.

## Do not restore verification against an empty rule set

Check what the endpoint is actually serving BEFORE fixing a mismatch:

    curl -s https://api.starstats.app/v1/parser-definitions | jq ".rules | length"

A client that cannot verify falls back to last-known-good. A client that CAN
verify adopts whatever it is handed, including an empty manifest, dropping the
rules it was running on. So a broken signature is, perversely, the only thing
protecting a fleet whose rules have gone missing server-side. Restore the
rules first (`scripts/restore-parser-rules.mjs`), then the key.

This exact sequence was live on 2026-08-27: 0 rules served, 5 still running in
the field, held there only by the signature mismatch.

## If a rotation looks like it failed

`--check` says MATCH but the manifest will not verify: the API was not
recreated. That is the answer nearly every time.

`--check` says MISMATCH: the write did not land. Re-run `--rotate`, or restore
the previous value from the 1Password item history.
