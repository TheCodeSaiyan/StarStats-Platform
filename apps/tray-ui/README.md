# tray-ui

Vite + React 19 + TypeScript webview that runs **inside** the Tauri
tray binary
([`crates/starstats-client`](../../crates/starstats-client/README.md)).
This is the UI for the tray menu, status pane, settings, and device
pairing.

## Hard rule: no direct network calls

The webview talks to the Rust backend **exclusively via Tauri IPC**.
There is no `fetch` to the StarStats API from this app, no embedded
bearer token, no direct contact with `robertsspaceindustries.com`.
All of that lives in the Rust side:

- The Rust backend holds the device JWT and the RSI session cookie
  (via the OS keychain).
- This UI calls Rust commands (defined in
  [`crates/starstats-client/src/commands.rs`](../../crates/starstats-client/src/commands.rs))
  and receives serialized data back.

Keeping the network out of the webview is what makes the tray's
secrets posture defensible. Don't break it.

## Stack

| Dep | Version | Notes |
|---|---|---|
| Vite | ^6.0 | Dev server + build |
| React | 19 | Concurrent features used liberally |
| TypeScript | ^5.7 | `tsc --noEmit` is the typecheck gate |
| `@tauri-apps/api` + plugin clients | ^2.x | IPC, notification, shell, process, updater |
| Vitest + Testing Library | ^1.6 / ^16 | Unit + render tests via `jsdom` |
| `@fontsource-variable/geist` & `geist-mono` | ^5.1 | UI fonts |
| `api-client-ts` | workspace | Shared types/paths from the server's OpenAPI spec, used for TS narrowing only |

The `api-client-ts` dep is for **types** — the tray doesn't issue HTTP
itself, but matching the server's schema makes IPC payloads type-safe
across the Rust ↔ TS boundary.

## Dev loop

Don't run this app standalone — the real dev loop spawns it as part
of the Tauri build. From the **repo root**:

```bash
pnpm install                  # one-off
pnpm tauri:dev                # vite dev server + tauri rebuild on save
```

Under the hood `tauri:dev` `cd`s into
[`../../crates/starstats-client`](../../crates/starstats-client/README.md)
and runs `pnpm exec tauri dev`, which in turn spawns
`vite` against this directory plus the Rust build. Hot-reload works
in both directions.

The package's own scripts are still useful in isolation:

```bash
pnpm --filter tray-ui dev          # vite dev server only (no Tauri)
pnpm --filter tray-ui build        # tsc -b && vite build
pnpm --filter tray-ui typecheck    # tsc --noEmit
pnpm --filter tray-ui lint         # ESLint
pnpm --filter tray-ui test         # Vitest watch mode
pnpm --filter tray-ui test:run     # Vitest single run
```

## UI conventions (selected)

These live more comprehensively in
[`../../docs/ENGINEERING.md`](../../docs/ENGINEERING.md); the ones most likely to bite
contributors here:

- **Event headlines route through `humanTitleForEntry`**
  (`format.ts`, mirrored web-side). Never surface raw snake_case
  `event_type` as a user-facing headline. The raw discriminant
  stays addressable as a tone-tinted chip + `title` tooltip + filter
  UI.
- **In-Transit movement noise is hidden at render-layer** —
  `timeline/filter.ts` in this app + `lib/event-filter.ts` in
  `apps/web` keep two suppressed sets in sync. Events still persist
  in the DB; the filter is display-only.
- **h1 type plateau:** baseline 28px from `globals.css`. Top-level
  pages opt to 32px inline; detail pages can opt to 24px inline.
  Don't override unless the depth tier calls for it.

## Related

- [`../../crates/starstats-client/README.md`](../../crates/starstats-client/README.md)
  — the Rust binary this UI is embedded in
- [`../../packages/api-client-ts/README.md`](../../packages/api-client-ts/README.md)
  — the generated types shared with this app
- [`../web/README.md`](../web/README.md) — the Next.js dashboard
  that shares filter logic and headline formatting with this app
