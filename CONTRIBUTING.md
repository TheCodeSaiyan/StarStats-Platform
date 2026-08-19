# Contributing to StarStats

Thanks for your interest. StarStats is a small, single-maintainer
project, but it's open source and contributions are welcome — bug
reports, fixes, parser additions, web/tray polish, all of it.

Before you start, please skim:

- [`CODE_OF_CONDUCT.md`](CODE_OF_CONDUCT.md) — Contributor Covenant 2.1
- [`EAC-SAFETY.md`](EAC-SAFETY.md) — the project's defining
  invariant. Anything that touches the running game process is out
  of scope and will be rejected on review.
- [`SECURITY.md`](SECURITY.md) — for security issues, *don't* open a
  public issue or PR; use the private channels documented there.

## Prerequisites

| Tool | Version | Why |
|---|---|---|
| Rust toolchain | **1.88** (workspace `rust-toolchain.toml` pin) | Workspace MSRV; some upstream deps (`aws-sdk-s3 1.110.0`) require it |
| `cargo-fmt`, `cargo clippy` | bundled with the toolchain | CI gates on both |
| Node.js | **20.x** | Web app + tray UI build |
| pnpm | **9.15+** | Workspace package manager |
| Postgres | **17** (with `pgvector`) *(only for server work)* | API server tests + dev |
| WebView2 *(Windows)* | latest | Tauri webview |
| GTK + WebKit2GTK *(Linux)* | distro packages | Tauri webview |

You don't need code-signing certs, registry credentials, or the
updater's minisign keypair — those live in GitHub Actions secrets and
only the release workflow uses them. Local builds skip signing.

## Cloning

```bash
git clone https://github.com/TheCodeSaiyan/StarStats-Platform.git
cd StarStats
```

The repository uses a Cargo workspace plus a pnpm workspace. The
`starstats-client` Tauri crate is excluded from default workspace
builds because it requires platform-specific webview deps; CI builds
it on dedicated matrix runners.

## Local-dev environment files

Two `.env.example` files exist — copy each next to its sibling:

```powershell
Copy-Item crates/starstats-server/.env.example crates/starstats-server/.env
Copy-Item apps/web/.env.example apps/web/.env.local
```

The defaults assume Postgres on `localhost:5432`. If you're not doing
server work, you can ignore the server `.env`.

## Build and test loops

### Rust workspace (server + core)

```bash
# Build, test, lint
cargo build  --workspace --exclude starstats-client
cargo test   --workspace --exclude starstats-client
cargo clippy --workspace --exclude starstats-client -- -D warnings
cargo fmt --all --check
```

CI runs all four against `starstats-core` and `starstats-server` on
every PR. `clippy -D warnings` is a hard gate; the workspace has
some allow-listed pedantic lints in the root `Cargo.toml` —
everything else fails.

### Tauri tray (full GUI loop)

You need two shells:

```bash
# Shell 1: Vite dev server for the tray UI
pnpm install
pnpm --filter tray-ui dev
```

```bash
# Shell 2: Rust backend (waits for the dev server URL)
cargo run -p starstats-client
```

Tauri picks up the dev server automatically via
`crates/starstats-client/tauri.conf.json`. For a release-style local
build:

```bash
pnpm --filter tray-ui build
cargo build -p starstats-client --release
```

### Web app

```bash
pnpm --filter web dev          # http://localhost:3000
pnpm --filter web build        # production build
pnpm --filter web start        # serve the production build
```

### Adjacent packages

```bash
# Regenerate the OpenAPI-derived TS client (CI fails if it drifts)
pnpm --filter api-client-ts run generate
```

## Branching and PR workflow

- `main` is the release branch. CI runs on every push and PR; tagged
  pushes (`v*`) trigger the release pipeline.
- Open feature branches off `main` (`feat/<short-name>`,
  `fix/<short-name>`, `docs/<short-name>` are all fine).
- Open a PR back into `main` when you're ready. Keep PRs focused —
  smaller is reviewed faster.
- Rebase on `main` before merge if there's drift; the project
  prefers a linear history but doesn't enforce squash.
- The default reviewer is the maintainer
  ([@ntatschner](https://github.com/ntatschner)). For security-sensitive
  changes (auth, signing, updater), expect more rounds of review.

## Commit conventions

The project uses [Conventional Commits](https://www.conventionalcommits.org/en/v1.0.0/).
Allowed types:

| Type | Use |
|---|---|
| `feat` | New user-visible feature |
| `fix` | Bug fix |
| `chore` | Maintenance — dep bumps, version bumps, scaffolding |
| `docs` | Documentation only |
| `refactor` | Code change that's neither a feature nor a fix |
| `test` | Test-only changes |
| `perf` | Performance improvement |
| `ci` | CI configuration changes |

Optional scopes are a small noun: `(parser)`, `(tray)`, `(server)`,
`(web)`, `(updater)`, `(deps)`. Examples from the existing log:

```
feat(parser): modern PlayerDeath + PlayerIncapacitated event types
feat(updater): per-channel manifests + Live/RC/Alpha selector
fix(storage): release conn lock between batches in for_each_event
fix(deps): bump tauri 2.11.0 → 2.11.1 for GHSA-7gmj-67g7-phm9
```

The first line should be ≤72 characters, lower-case after the type
prefix, no trailing full stop. Add a body if the *why* needs more
than the subject line carries.

## Style and quality bar

- **rustfmt:** all Rust code must be `cargo fmt --check` clean.
- **clippy:** `-D warnings` against `starstats-core` and
  `starstats-server`. The workspace allows a small handful of
  pedantic lints — see `[workspace.lints.clippy]` in the root
  `Cargo.toml` for the list and the rationale.
- **Tests:** add or update tests near the change. The Rust crates
  use `cargo test`; the web app uses Vitest; the tray UI uses
  whatever its package configures.
- **Docs:** keep `docs/ARCHITECTURE.md` current when a structural
  change lands. Update [`CHANGELOG.md`](CHANGELOG.md)'s
  `[Unreleased]` section as part of the same PR.
- **Comments:** match the voice of the existing codebase — the
  `crates/starstats-core/src/events.rs` doc-comments are a good
  reference. We prefer "why" over "what"; the type signature already
  tells you what.

## Adding a new `Game.log` event variant

This is the most common contribution. Rough recipe:

1. Capture a real `Game.log` line (or several) that exhibits the
   pattern. Strip personal handles if you're sharing in a PR.
2. Add a struct + a `GameEvent` variant in
   `crates/starstats-core/src/events.rs`.
3. Add a regex + classifier branch in
   `crates/starstats-core/src/parser.rs`. Prefer adding a new entry
   to the dynamic parser-defs table where possible.
4. Add fixtures and tests under `crates/starstats-core/tests/`.
5. If the event surfaces in the tray UI, wire it into
   `apps/tray-ui`. If it surfaces in the API, add the corresponding
   server route under `crates/starstats-server`.
6. Update `CHANGELOG.md` `[Unreleased] / Added`.

`Game.log` is the source of truth — the parser must never claim
something the line doesn't actually say.

## Releases

Release artifacts are produced by
[`.github/workflows/release.yml`](.github/workflows/release.yml),
which fires on tag pushes matching `v*`. Tag suffix selects the
update channel:

| Tag | Channel | Manifest committed back to `main` |
|---|---|---|
| `vX.Y.Z-alpha[.N]` | Alpha | `release-manifests/tray-alpha.json` |
| `vX.Y.Z-rc[.N]` | RC | `release-manifests/tray-rc.json` |
| `vX.Y.Z` (bare semver) | Live | `release-manifests/tray-live.json` |

The maintainer cuts releases. Contributors don't need to do
anything — open the PR, get it merged, and the next release picks
your change up. Local builds never touch the signing key or the
release manifests.

## Reporting bugs

- **Tray / parser bugs:** open an issue with the `Game.log` line(s)
  involved (redact handles you don't want public), the StarStats
  version (Settings → About), the OS, and the channel you're on.
- **Server / web bugs:** include the request ID from the response
  headers if you have one, and the rough timestamp.
- **Security bugs:** use the channels in
  [`SECURITY.md`](SECURITY.md). Do not file a public issue.

## Code of conduct

By participating in this project you agree to abide by the
[Contributor Covenant 2.1](CODE_OF_CONDUCT.md). Reports go to
<conduct@starstats.app>.

Welcome aboard. Stay clear of the game process.
