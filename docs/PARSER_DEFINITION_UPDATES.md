# Dynamic parser-definition updates — design note

> **Status:** design only. No implementation yet. Tracked for v0.3.x.

## Why

The Star Citizen client log vocabulary changes between patches. Today every
new `<EventName>` regex requires a tray-app rebuild + signed installer + user
re-install. That cycle is too slow for a community-driven project where:

- A user notices a new line shape we haven't recognised yet.
- They (or a maintainer) write a regex for it.
- Every other user gains the recognition without needing a new build.

## Goals

1. **Append-only**: remote rules can *add* recognition, not override or
   suppress built-in classifiers. The compiled-in `parser::classify`
   stays authoritative — remote rules run only when `classify` returned
   `None`.
2. **Trustable**: rules are signed by a maintainer key. Clients reject
   unsigned manifests so a hijacked CDN can't inject malicious patterns.
3. **Cache-friendly**: cache the active manifest in SQLite so an offline
   client stays at parity with its last successful fetch.
4. **Inspectable**: every remote-matched event in the local store is
   annotated with the rule id + version it was matched by, so a buggy
   rule can be retracted without rebuilding the client.

## Wire shape

`GET /v1/parser-definitions`

```json
{
  "version": 1,
  "schema_version": 1,
  "issued_at": "2026-05-07T12:00:00Z",
  "rules": [
    {
      "id": "abc123",
      "event_name": "PlayerExitedShipFromCockpit",
      "body_regex": "Player\\[(?P<player>[^\\]]+)\\].*?vehicle\\[(?P<vehicle>[^\\]]+)\\]",
      "output_type": "remote_match",
      "fields": [
        { "name": "player",  "from": "player" },
        { "name": "vehicle", "from": "vehicle" }
      ],
      "min_client_version": "0.3.0"
    }
  ],
  "signature": "<base64 ed25519 sig over the canonicalised rules array>"
}
```

## New `GameEvent` variant

Add `GameEvent::RemoteMatch(RemoteMatch)` so the wire format stays
consistent and the validators / sync-batcher don't need a special case.

```rust
pub struct RemoteMatch {
    pub timestamp: String,
    pub rule_id: String,
    pub event_name: String,
    pub fields: BTreeMap<String, String>,
}
```

## Apply order

In `parser::classify`:

1. Built-in `match event` shell dispatch (current code).
2. Built-in body-prefix dispatch (`classify_body_prefix`).
3. **New:** remote-rule dispatch — iterate compiled remote rules, match
   `event_name` if shell present, otherwise scan body for the rule's
   keyword. First match wins.

## Client fetcher

- `crates/starstats-client/src/parser_defs.rs`
- Fetches `GET /v1/parser-definitions` on startup + every 6h.
- Verifies the ed25519 signature against an embedded maintainer pubkey.
- Persists to a new `remote_parser_rules` SQLite table.
- On startup, loads the cached manifest if the network is unavailable.

## Server endpoint

- `crates/starstats-server/src/parser_routes.rs`
- Stores manifests in S3/MinIO; the endpoint serves the latest signed
  manifest. Maintainers PUT new manifests via an authenticated admin
  route (out of scope for this design — assume manual upload for v1).

## Open questions

- Should rules be channel-scoped (LIVE vs PTU)? Probably yes — PTU log
  shapes drift before they reach LIVE.
- Should we expose a per-user "I don't trust remote rule X" opt-out?
  Defer until we have a real instance of this.
- Submission flow (community → maintainer review): out of scope for v1;
  a forms-style PR-style workflow is the right answer but it's its own
  feature.

---

## Recovery: the served rule set has no source of truth outside the database

**Status as of 2026-08-27:** `GET /v1/parser-definitions` serves **0 rules**.
A tray that cached the manifest on 2026-07-21 holds **5**. The rules are gone
from the server.

### Why this could happen silently

`migrations/0048_parser_rules.sql` creates the table and inserts nothing. The
only write path is a moderator publishing an approved submission through
`admin_parser_rules.rs`. So the served rule set exists **only** as
hand-published rows: no seed, no fixture, nothing in version control. Lose the
rows — a restore, a wrong environment, a manual delete — and there is no copy
to compare against and nothing that notices.

It is also invisible from the outside. An empty rule set and a healthy one are
both `200`. (A rule-load *failure* is now `503` rather than an empty manifest,
so at least the two are distinguishable — see the note on `current_manifest`.)

### The recovered set

`docs/recovery/parser-rules-2026-07-21.json` holds all five, verbatim, with
their `body_regex` and `fields`, recovered from a tray's local
`parser_def_manifest` cache:

| rule id | event name |
| --- | --- |
| `asop.fetch_vehicles.v1` | `OnRequestFetchVehicles` |
| `comms.notification.v1` | `SHUDEvent_OnCommsNotification` |
| `party.marker.v1` | `CPartyMarkerComponent RWES` |
| `shop.buy.standard.v1` | `CEntityComponentShoppingProvider::SendStandardItemBuyRequest` |
| `shop.buy.ui.v1` | `CEntityComponentShopUIProvider::SendShopBuyRequest` |

That file is a **recovery record, not a seed** — nothing reads it.

### How to restore

The admin UI cannot do it. `/admin/parser-rules` lists published rules and
toggles them by re-POSTing their own fields as hidden inputs, so against an
empty table it shows an empty list and offers no way to create one. There is
no "new rule" page (unlike `/admin/parser-inference-rules/new`).

So it goes through the API the UI itself calls, `POST /v1/admin/parser-rules`,
which needs a **moderator** token.

Save the token to a file first, and **do not put it on the command line** —
arguments and inline `VAR=... cmd` prefixes land in shell history and are
visible in the process list:

```
# paste the token into ./tok (git-ignored, delete it afterwards)
node scripts/restore-parser-rules.mjs --dry-run          # no token needed
node scripts/restore-parser-rules.mjs --token-file ./tok
rm ./tok
```

The file may contain **either** a bare JWT **or** the whole
`starstats_session` cookie exactly as devtools copies it — that cookie is
URL-encoded JSON (`%7B%22t%22%3A%22eyJ...`), which is the value you actually
have to hand, so the script decodes it and takes the `t` field itself.

It upserts each rule by `rule_id` (so re-running is safe), then re-reads
`GET /v1/parser-definitions` — the endpoint clients actually consume — to
confirm the count came back. It never prints the token.

**On JWT exposure.** These are stateless: `auth.rs` checks only *device*
tokens against a revocation store, so a leaked `token_type: "user"` token
stays valid until `exp` and signing out does not invalidate it. If one is
exposed, the realistic mitigations are to wait out the (short) expiry and
clear it from shell history.

### Restore before fixing the signing key, not after

These interact, and the order matters. The client rejects the live manifest
because the server's signing key no longer matches the pubkey pinned in
`parser_defs.rs` (a manifest cached 2026-07-21 verifies against the pin;
today's does not). **That rejection is the only reason trays are still running
the 5 rules** — they fall back to last-known-good.

Fix the key while the table is still empty and every tray will happily verify
and adopt an empty manifest, dropping the rules they are currently running on.
Repopulate first, then fix the key.

### Worth closing properly

- Nothing alerts when the served rule count drops to zero.
- There is no export, so this recovery depended on one machine's cache.
