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
