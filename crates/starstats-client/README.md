# starstats-client

Tauri 2 tray app for [StarStats](../../README.md) — the Rust backend
that tails `Game.log`, buffers events in a local SQLite store,
optionally scrapes the user's RSI profile + hangar pages, and drains
events upstream to the API server when sync is enabled.

`starstats-client` is **excluded from the default Cargo workspace
build** because Tauri needs platform-specific system dependencies.
CI builds it on dedicated matrix runners; locally you opt in with
`-p starstats-client`.

## What lives here

| Module | Role |
|---|---|
| `gamelog.rs` | `Game.log` file watcher (via `notify`) and tailing loop |
| `storage.rs` | Bundled SQLite (`rusqlite` w/ `bundled`) — event buffer, sync cursors, noise list, pairing state |
| `sync.rs` | Drains the queue in batches → `POST /v1/ingest` with the device JWT |
| `discovery.rs` | Resolves the API base URL (server URL discovery) |
| `commands.rs` | Tauri IPC commands surfaced to the React webview |

Other notable bits: a hangar fetcher that scrapes
`robertsspaceindustries.com/account/pledges` using the user's own
RSI session cookie, gated by an EAC-aware scheduling guard
(`sysinfo` skips cycles while `StarCitizen.exe` is running so two
authenticated HTTP sessions from the same machine don't trip EAC
heuristics). The shared parser and `GameEvent` enum live one crate
over in
[`starstats-core`](../starstats-core/README.md).

## Secrets posture

- The **RSI session cookie** is the only credential the tray ever
  holds. It lives in the OS keychain (Windows Credential Manager /
  macOS Keychain / Linux Secret Service via `dbus-secret-service`
  + RustCrypto — `keyring` 3.x with the matching feature set in
  `Cargo.toml`).
- The **device JWT** issued by the server during pairing is also
  stored locally; revocation is enforced server-side via
  `DeviceStore` on every protected request, so a stolen token
  becomes useless the moment the user revokes the device.
- The server never sees the RSI cookie. This is an architectural
  invariant — see [`../../docs/ENGINEERING.md`](../../docs/ENGINEERING.md) §Architecture
  Invariants.

## Building

### Windows

WebView2 ships with Windows 10+, so no extra system deps:

```powershell
cargo build -p starstats-client
```

### Linux (Debian/Ubuntu)

```bash
sudo apt install libwebkit2gtk-4.1-dev libgtk-3-dev \
                 libayatana-appindicator3-dev librsvg2-dev \
                 libssl-dev pkg-config
cargo build -p starstats-client
```

### macOS

Tauri targets macOS but CI doesn't currently build for it. Local
builds should work; ping the maintainer if you hit Tauri 2 specifics.

## Dev loop

From the repo root (`StarStats/`):

```bash
pnpm install                  # one-off; pulls tray-ui webview deps
pnpm tauri:dev                # vite dev server + tauri rebuild on save
```

That script `cd`s into this crate and runs `pnpm exec tauri dev`,
which spawns the Vite dev server for [`apps/tray-ui`](../../apps/tray-ui)
and a hot-reloading Rust build. Stop everything with one Ctrl-C.

To produce a release installer:

```bash
pnpm tauri:build
```

## Tauri plugins enabled

The crate wires up the official Tauri 2 plugins relevant to the tray's
runtime needs:

| Plugin | Why |
|---|---|
| `tauri-plugin-shell` | Open external URLs (release notes, dashboard) |
| `tauri-plugin-notification` | Pairing prompts, sync errors, update-available toasts |
| `tauri-plugin-single-instance` | Refuse to launch a second tray when one is already running |
| `tauri-plugin-updater` | Polls the per-channel manifest and prompts to install |
| `tauri-plugin-process` | Process lifecycle helpers (restart on update apply) |
| `tauri-plugin-autostart` | Optional "launch on login" toggle |

## Logging

Release builds run with `windows_subsystem = "windows"`, so stdout
is detached. The tray writes a daily-rolling file via
`tracing-appender` so a setup-time panic is diagnosable without
rebuilding a debug binary. The log path is documented in
[`../../docs/AUTOSTART.md`](../../docs/AUTOSTART.md).

## Icons

Platform icons live in `icons/` — `32x32.png`, `128x128.png`,
`icon.png` / `icon.ico` / `icon.icns`. Tauri picks the right one
per bundle target.

## Related

- [`../starstats-core/README.md`](../starstats-core/README.md) —
  shared `GameEvent` enum, parser, wire format
- [`../starstats-server/README.md`](../starstats-server/README.md) —
  the API the tray syncs to
- [`../../apps/tray-ui/README.md`](../../apps/tray-ui/README.md) —
  the React webview that this binary embeds
- [`../../docs/ARCHITECTURE.md`](../../docs/ARCHITECTURE.md) —
  end-to-end system design
