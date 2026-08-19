# StarStats

[![Live release](https://img.shields.io/badge/live-v1.3.2-green)](https://github.com/TheCodeSaiyan/StarStats-Platform/releases/tag/v1.3.2)
[![License](https://img.shields.io/badge/license-MPL--2.0-blue)](LICENSE)
[![Latest release](https://img.shields.io/github/v/release/TheCodeSaiyan/StarStats-Platform?include_prereleases&sort=semver)](https://github.com/TheCodeSaiyan/StarStats-Platform/releases)

EAC-safe personal metrics for Star Citizen. A tray client + API + web
companion that turns the data the game already writes about you into
something you can actually look at.

**<https://starstats.app>**

> **Status: live channel at `v1.3.2`.** The shape is settled and the
> live channel is what new users should pick. Channels (`live` /
> `rc` / `beta` / `alpha`) are opt-in update tracks, **not** semver
> phases — `live` is the recommended default, `rc` mirrors release
> candidates, and `beta` / `alpha` may sit on older snapshots while
> work that targets `live` lands first. The wire format and OpenAPI
> spec are still allowed to break on minor releases until a `2.0`
> commits to stability.

## What it is

StarStats reads two things, and only those two things:

1. **Your local `Game.log` file.** The game writes this itself, on
   every session, to a world-readable text file under
   `%LOCALAPPDATA%\..\StarCitizen\LIVE\Game.log` (or your equivalent
   `LIVE` / `PTU` / `EPTU` / `TECH-PREVIEW` install). The tray client
   tails it the same way Notepad would.
2. **`robertsspaceindustries.com`, as you.** If you paste your own RSI
   session cookie into the tray's Hangar Sync settings, StarStats
   issues authenticated HTTPS requests to your own profile and hangar
   pages — exactly what your browser already does when you're logged
   in.

That's the whole input surface. There is no DLL, no overlay, no
process attach, no memory read, no game-network sniffing, no macros,
no automation of in-game actions.

## EAC-safety, in one paragraph

Easy Anti-Cheat watches for tampering with the running game process
and its memory, modules loaded into the game, hooks against game
APIs, and modifications of game files. StarStats does **none** of
those things. It opens a text file the game flushed to disk, and it
issues plain HTTPS requests to a public website you're already logged
into. The Windows APIs involved are the ones used by every editor,
browser, and chat client on your machine. The full technical
explanation — including a side-by-side comparison with overlay and
memory-reading tools that *do* get bans — lives in
[`EAC-SAFETY.md`](EAC-SAFETY.md). If you have any doubt before
running it, read that file first.

## What it tracks today

Parsed from `Game.log` (`crates/starstats-core/src/events.rs` is the
authoritative list):

| Lifecycle | Combat | Economy | Movement | Operational |
|---|---|---|---|---|
| `ProcessInit` / `LegacyLogin` | `PlayerDeath` | `ShopBuyRequest` / `ShopFlowResponse` | `JoinPu` / `ChangeServer` | `HudNotification` |
| `MissionStart` / `MissionEnd` | `PlayerIncapacitated` | `CommodityBuyRequest` / `CommoditySellRequest` | `SeedSolarSystem` / `ResolveSpawn` | `AttachmentReceived` |
| `SessionEnd` / `GameCrash` | `ActorDeath` *(legacy)* | (transaction pairing into completed orders) | `QuantumTargetSelected` | `LocationInventoryRequested` |
| `LauncherActivity` | `VehicleDestruction` | | `PlanetTerrainLoad` / `VehicleStowed` | `RemoteMatch` |

Pulled from RSI (when you've pasted your session cookie):

- Public-profile snapshot (handle, citizen number, enlisted-since).
- Hangar inventory diff (what came/went between checks).

## Key features

- **Auto-update with channel selector.** The tray polls a per-channel
  manifest on `main` (`release-manifests/tray-{alpha,beta,rc,live}.json`)
  and prompts to install. Switch channels in Settings; tag suffix
  decides which manifest CI updates (`-alpha` → alpha, `-beta` →
  beta, `-rc` → rc, bare semver → live).
- **Re-parse.** When the parser learns a new event type, hit
  *Re-parse* to re-classify everything already in your local store
  in place — no log-file replay needed.
- **Transaction pairing.** Buy and sell requests are matched against
  their flow-response confirmations into a single completed order
  with price, quantity, and location resolved.
- **Burst collapse.** Spammy multi-line bursts in `Game.log` —
  loadout-restore shower, terrain-load blast, jurisdiction HUD
  stutters, hangar vehicle-stowed runs — fold into one
  `BurstSummary` row per group via the deterministic
  template/burst matcher in `crates/starstats-core/src/templates.rs`.
  Members are suppressed at ingest; the web timeline renders the
  summary with a friendly per-rule label and event count.
- **Web companion.** A Next.js dashboard that surfaces the same data
  with a longer history view than the tray affords.
- **Cloud sync for preferences (v1.5).** Per-device opt-in. Pair the
  tray to your account, flip the *Cloud sync* toggle, and your theme
  + sync cadence ride along to every uplink you turn on. Revoke any
  uplink remotely from `/devices` (Connected Uplinks). Web is always
  your account view when signed in; clearing site data no longer
  loses your settings.
- **Records widget (v1.5).** Your deadliest single session surfaced
  on the dashboard with a click-through to the underlying timeline.
  Sits alongside the activity heatmap and top-event-types
  distribution, so context is one hop away.
- **Granular sharing.** Per-event visibility on top of profile-level
  controls: public, RSI org-only, named-handle grants (with expiry +
  notes), or fully private. Inbound shares land on a dedicated
  `/sharing` page with a nav badge for unread.
- **Org workspace.** RSI org owners get a shared dashboard with
  member roles enforced by Zanzibar-style ReBAC (SpiceDB), so per-org
  privacy is a relationship check rather than a row filter.
- **EAC-safe by construction.** See
  [`EAC-SAFETY.md`](EAC-SAFETY.md). It's the entire reason this
  project exists.

## Install (users)

Pre-built installers ship on every tagged release:
**<https://github.com/TheCodeSaiyan/StarStats-Platform/releases>**

| Platform | Artifact | Notes |
|---|---|---|
| Windows | `StarStats_<version>_x64-setup.exe` (NSIS) or `.msi` (WiX) | Code-signed; auto-update enabled |
| Linux | `StarStats_<version>_amd64.AppImage` or `.deb` | AppImage is portable; `.deb` for Debian/Ubuntu |

After install:

1. Launch StarStats. It runs to the system tray; click the icon for
   the menu.
2. Open *Settings* and point *Game.log path* at your install's
   `Game.log`. The default Windows location is
   `%LOCALAPPDATA%\..\StarCitizen\LIVE\Game.log` — adjust for `PTU`
   etc. The picker only allows files named `Game.log`.
3. *(Optional)* Paste your RSI session cookie into *Hangar Sync* if
   you want hangar/profile snapshots. The cookie is stored in your OS
   keychain (Windows Credential Manager / macOS Keychain / Linux
   Secret Service) — only the same OS user that pasted it can read
   it back.
4. Launch Star Citizen. Events flow into the tray as the game writes
   them. The history pane and the *Re-parse* button live under the
   *Events* tab.

## Build (contributors)

Full developer setup, branching model, commit conventions, and the
release flow live in [`CONTRIBUTING.md`](CONTRIBUTING.md). Quick
start:

```powershell
# Prereqs: Rust 1.88, pnpm 9.15, Node 20.18+.
# For server work: Postgres 17 (with pgvector). For tray work: WebView2 (Windows) or GTK + WebKit2GTK (Linux).

# Clone
git clone https://github.com/TheCodeSaiyan/StarStats-Platform.git
cd StarStats

# Rust workspace (excludes the Tauri client by default — see CONTRIBUTING.md for the GUI loop)
cargo build  --workspace --exclude starstats-client
cargo test   --workspace --exclude starstats-client
cargo clippy --workspace --exclude starstats-client -- -D warnings
cargo fmt --all --check

# Web app
pnpm install
pnpm --filter web dev

# Tray (full GUI loop)
pnpm --filter tray-ui dev   # in one shell
cargo run -p starstats-client   # in another, after the dev UI is up
```

Code signing, updater signing, and registry credentials are all
supplied via GitHub Actions secrets — contributors never need any of
those locally. CI runs `cargo fmt --check`, `cargo clippy -D
warnings`, `cargo test`, and the web/tray TS builds on every PR.

## Repository layout

```
StarStats/
├── Cargo.toml                  Rust workspace
├── pnpm-workspace.yaml         pnpm + Turborepo
├── crates/
│   ├── starstats-core/         shared types, parser, validators
│   ├── starstats-server/       API server (Axum + sqlx)
│   └── starstats-client/       Tauri tray app (Rust backend)
├── apps/
│   ├── tray-ui/                Tauri webview frontend (Vite + React)
│   └── web/                    Next.js dashboard
├── packages/
│   └── api-client-ts/          generated TS client (from server's OpenAPI spec)
├── infra/                      docker-compose configs for local development
├── docs/
│   ├── ARCHITECTURE.md         end-to-end system design
│   ├── OBSERVABILITY.md        per-component telemetry matrix
│   └── AUDIT.md                tamper-evident audit log design
├── release-manifests/          per-channel updater manifests (alpha/rc/live)
    └── python-spike/           original Python prototype, retained for reference
```

## What StarStats does NOT do

- Read game memory.
- Inject into the game process.
- Hook game APIs.
- Sniff or modify game network traffic.
- Modify any game files.
- Drive in-game input (no macros, no aimbots, no multiboxing
  automation).
- Touch other players' data — only your own log file and your own
  RSI session.

If a feature would require any of the above, it doesn't belong in
StarStats and won't be merged.

## Documentation

- [`EAC-SAFETY.md`](EAC-SAFETY.md) — why this is safe, in technical
  detail with a side-by-side comparison vs. overlay / memory-reading
  tools
- [`SECURITY.md`](SECURITY.md) — coordinated-disclosure policy and
  release-artifact verification
- [`CONTRIBUTING.md`](CONTRIBUTING.md) — dev setup, branching model,
  commit conventions, "Adding a new `Game.log` event variant" recipe
- [`CHANGELOG.md`](CHANGELOG.md) — release history (Keep a Changelog
  format, populated through v1.2.1)
- [`CODE_OF_CONDUCT.md`](CODE_OF_CONDUCT.md) — Contributor Covenant 2.1
- [`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md) — end-to-end system
  design, threat model, dataflow, auth, storage
- [`docs/OBSERVABILITY.md`](docs/OBSERVABILITY.md) — per-component
  telemetry matrix
- [`docs/AUDIT.md`](docs/AUDIT.md) — hash-chained audit log, MinIO
  mirror, retention
- [`docs/ENGINEERING.md`](docs/ENGINEERING.md) — working agreements, conventions, do/don't
  (contributor-facing companion to `CONTRIBUTING.md`)
- [`NOTICE`](NOTICE) — third-party acknowledgements
- Per-package READMEs under `crates/`, `apps/`, and `packages/`

## Project status, honestly

StarStats is pre-1.0 alpha software run by a single maintainer. The
parser learns new event types as Star Citizen patches drop, and the
tray, server, and web app are all still being shaped. Things that
explicitly aren't done yet:

- The `starstats.app` website is registered but not deployed — links
  to it in this README are forward-looking.
- macOS builds. Tauri targets it; CI doesn't yet.
- A formal stable API. Until 1.0 the OpenAPI spec is allowed to break
  on minor releases.

When something breaks, file an issue:
<https://github.com/TheCodeSaiyan/StarStats-Platform/issues>. For security
problems, use the channels in [`SECURITY.md`](SECURITY.md) — not
public issues.

## Licence

Licensed under the Mozilla Public License, v. 2.0. See
[`LICENSE`](LICENSE) for the full text. MPL-2.0 is file-level
copyleft: modifications to existing source files must be released
under the same terms, but the code can be combined with proprietary
or other-licensed code in larger works.

Star Citizen is a trademark of Cloud Imperium Games. StarStats is not
affiliated with, endorsed by, or sponsored by Cloud Imperium Games or
the Roberts Space Industries website.
