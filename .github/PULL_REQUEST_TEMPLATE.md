<!--
  PR title should follow Conventional Commits — match the project's style:
    feat(parser): ...
    fix(tray): ...
    chore(deps): ...
    fix(release): ...
  See `git log --oneline` for examples.
-->

## What changed and why

<!-- One or two paragraphs. Lead with the user-visible effect, then the
     mechanism. If this is a refactor with no behaviour change, say so
     explicitly. -->

## Linked issues

Fixes #
<!-- or: Refs #, Part of #. Leave blank if exploratory. -->

## Tests

- [ ] `cargo test -p starstats-core -p starstats-server` passes locally
- [ ] `pnpm -F tray-ui test` passes locally (or n/a — no tray UI tests touched)
- [ ] `pnpm -F web test:e2e` passes locally (or n/a — no web changes)
- [ ] Manually verified — describe what you clicked through:
  <!-- e.g. "Opened tray, switched channel to RC, hit Re-parse, watched the
       Coverage card update." Required for UI-only changes. -->

## Lint & formatting

- [ ] `cargo fmt --all -- --check` clean
- [ ] `cargo clippy -p starstats-core -p starstats-server -- -D warnings` clean
- [ ] `pnpm --filter web lint` clean (if web touched)
- [ ] `pnpm --filter web typecheck` and/or `pnpm --filter tray-ui typecheck` clean (if TS touched)

## Screenshots / GIFs

<!-- Required for any change that affects the tray UI or the web app.
     Drag-and-drop directly into the PR. -->

## EAC safety check

- [ ] This change does **not** add any code that:
  - reads or writes the running `StarCitizen.exe` process memory,
  - hooks Windows input, the SC renderer, or any DLL,
  - draws an overlay on top of the game window,
  - modifies any file under the SC install directory or `Game.log` itself,
  - bypasses or spoofs anything Easy Anti-Cheat would reasonably flag.

  StarStats remains **read-only `Game.log` tail + authenticated RSI page
  scraping only**. If you're not sure whether a change crosses this line,
  flag it in the PR description and we'll discuss before merge — or send a
  private note via the address in [SECURITY.md](../blob/main/SECURITY.md).

## Anything else reviewers should know

<!-- Migrations, follow-up work, known limitations, related PRs. -->
