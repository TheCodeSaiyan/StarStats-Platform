# starstats-core

Shared types, log parser, wire format, and validators for
[StarStats](../../README.md). Pulled in by **both** the tray client
(`starstats-client`) and the API server (`starstats-server`), so the
exact same `GameEvent` definitions, parsing logic, and serialization
shape cross the network on every ingest batch. A parser change here
cannot cause client/server skew.

## Module map

| Module | What it owns |
|---|---|
| `events.rs` | The canonical `GameEvent` enum — every event variant the project understands, in one place |
| `parser.rs` | Two-pass `Game.log` parser: a structural pass extracts `timestamp / level / event-name / rest`; a classify pass owns the per-variant regex tree |
| `wire.rs` | Wire format types — `IngestBatch`, `IngestResponse`, ingest envelope, idempotency keys |
| `validators.rs` | Shared validation rules — handle format, batch size limits, etc. |
| `templates.rs` | Burst-collapse template engine (deterministic matcher for spammy multi-line log bursts) |

## Why a separate crate?

The two-pass parser keeps adding new event variants safe: the
structural pass is stable, so a new event type only touches the
classify pass and the `GameEvent` enum. Because both the tray and
the server depend on this crate (the tray emits the events; the
server validates them on ingest), promoting a new variant lands in
one PR rather than two coupled-but-separate changes.

Adding a variant typically looks like:

1. Add it to `GameEvent` in `events.rs` (with the right serde tag).
2. Add a classifier branch in `parser.rs::classify`.
3. Add a fixture in `tests/` plus a proptest if structure is fuzzy.
4. (Server side) decide whether the new variant needs special
   handling in `ingest.rs`; most don't.

## Testing

Heavy use of [`proptest`](https://docs.rs/proptest/) for parser
properties and [`pretty_assertions`](https://docs.rs/pretty_assertions)
for readable diffs on regression fixtures (both in `[dev-dependencies]`).

```bash
cargo test -p starstats-core
```

The crate is part of the default workspace build, so the usual
`cargo test`, `cargo clippy -- -D warnings`, and `cargo fmt --check`
all cover it without extra flags.

## Backward-compatible parsing

Wire-format structs use `#[serde(default)]` on additive fields so
older tray builds can keep ingesting after the server learns a new
optional field. This is one half of an invariant pair — the other
half is that database migrations are append-only and byte-immutable
post-deploy (see [`../../docs/ENGINEERING.md`](../../docs/ENGINEERING.md) §Architecture
Invariants).

## Related

- [`../starstats-client/README.md`](../starstats-client/README.md) —
  consumes the parser to tail `Game.log` and emits batches
- [`../starstats-server/README.md`](../starstats-server/README.md) —
  consumes the wire format to validate and store ingest batches
- [`../../docs/ARCHITECTURE.md`](../../docs/ARCHITECTURE.md) §Data flow
- [`../../docs/PARSER_DEFINITION_UPDATES.md`](../../docs/PARSER_DEFINITION_UPDATES.md)
  — process for adding new event types
