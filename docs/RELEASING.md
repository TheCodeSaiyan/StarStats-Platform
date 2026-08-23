# Releasing StarStats

Operator manual for shipping StarStats. Design rationale:

- `the release design notes`
  (branch model, channel concept, validate-tag invariants).
- `the release design notes` (tray vs
  platform split — the change reflected in this revision of the manual).

---

## 1. Overview

Since 2026-05-23 StarStats ships on **two independent tracks**:

- **`tray`** — Tauri desktop client (signed MSI / AppImage, in-app
  auto-update). Tags: `tray-vX.Y.Z[-channel.N]`.
- **`platform`** — server + web container images (homelab Komodo
  deploy). Tags: `vX.Y.Z[-channel.N]` (unchanged from pre-split).

Each track has its own version number, its own bump cadence, and its
own auto-alpha trigger. A web-only change ships a new `platform`
version without touching `tray`; a tray-side bug fix ships a new
`tray` version without forcing a server redeploy.

Within each track the **four channels** are unchanged:

| Channel | Tray tag shape             | Platform tag shape        | Audience                  |
|---------|----------------------------|---------------------------|---------------------------|
| `alpha` | `tray-vX.Y.Z-alpha[.N]`    | `vX.Y.Z-alpha[.N]`        | Internal smoke / dogfood  |
| `beta`  | `tray-vX.Y.Z-beta[.N]`     | `vX.Y.Z-beta[.N]`         | Opt-in testers            |
| `rc`    | `tray-vX.Y.Z-rc[.N]`       | `vX.Y.Z-rc[.N]`           | Release candidates        |
| `live`  | `tray-vX.Y.Z` (bare)       | `vX.Y.Z` (bare)           | Default, all users        |

The repo has **two long-lived branches** (unchanged):

- **`main`** — IS the live channel. Anything merged here is on its way to
  every user on the default channel of both tracks.
- **`next`** — pre-release integration. Feature branches PR into `next`.
  Pre-release tags (both tracks) are cut from `next` SHAs.

Channel is selected by **tag suffix**. The `validate-tag` CI job in
both `release.yml` (tray) and `release-images.yml` (platform) enforces
that the suffix matches the branch the tag sits on.

The Tauri updater polls
`release-manifests/tray-{alpha,beta,rc,live}.json` on
`raw.githubusercontent.com/<owner>/<repo>/main/...`. All manifests live
on `main`; CI checks `main` out, commits the new manifest, and pushes.
**Only the tray track has auto-update manifests** — the platform
(server + web) is operator-deployed via Komodo, not user-installed.

### Version surfaces

| Track | Version file(s) | Notes |
|---|---|---|
| `tray`     | `crates/starstats-client/Cargo.toml` + `crates/starstats-client/tauri.conf.json` | Decoupled from workspace.package since the split; the client crate carries its own literal `version` field. |
| `platform` | `[workspace.package].version` in root `Cargo.toml` | Inherited by `starstats-core` + `starstats-server` via `version.workspace = true`. The `web` image tag uses this version. |

---

## 2. Branch model + invariants

These four invariants are load-bearing. Don't violate them; the CI guards
will reject your push anyway.

1. **`main` is a strict ancestor of `next` at rest.** After every commit
   that lands on `main` (promotion OR hotfix), `main` is merged into
   `next` so `next` never falls behind. The promotion script enforces
   this for promotions; for hotfixes you run `hotfix-finish`.
2. **Feature PRs target `next`.** The only PRs into `main` are the
   promotion PR (`next` → `main`, usually a fast-forward push from the
   script) and `hotfix/*` → `main`.
3. **Tag-branch coupling.**
   - `vX.Y.Z-{alpha,beta,rc}[.N]` must sit on a commit reachable from
     `origin/next` AND **not** reachable from `origin/main`. (If the
     commit is already on `main` it's already live — pre-release tag is
     nonsensical.)
   - `vX.Y.Z` must sit on a commit reachable from `origin/main`.
4. **All `release-manifests/*.json` live on `main`.** The
   `Update channel manifest on main` step in `release.yml` checks `main`
   out, pulls `--ff-only`, commits, pushes. Never edit channel manifests
   on `next` or in a feature branch.

`Cargo.toml` and `apps/desktop/src-tauri/tauri.conf.json` versions MUST
match the tag's bare semver (i.e. strip any `-alpha.N` suffix). The
manifest script fails the build if they don't. `release-promote.mjs`
handles the bump for you.

---

## 3. Daily flow: feature → `next`

```bash
git fetch origin
git switch -c feat/widget-x origin/next

# ... hack, commit, push ...

gh pr create --base next --head feat/widget-x \
  --title "feat(widgets): add widget X" \
  --body "..."
```

Notes:

- Default base of any new PR is `next`. If you accidentally open against
  `main`, edit the PR base before merging — the only PRs into `main` are
  promotions and hotfixes.
- `ci.yml` runs on both `main` and `next`. Same Rust + web + tray-ui +
  Tauri client + OpenAPI-drift + Playwright matrix.
- Merge style on `next`: rebase, don't merge.

---

## 4. Ship a pre-release

From a clean checkout with `next` at the SHA you want to ship:

```bash
git fetch origin
git switch next
git pull --ff-only origin next

node scripts/release-promote.mjs prerelease beta
```

What it does:

1. Reads the current version from `Cargo.toml`.
2. Computes the next pre-release version. If `Cargo.toml` already has a
   matching suffix (e.g. `1.9.0-beta.1`), bumps `.N`. Otherwise stamps
   the requested channel at `.1`.
3. Rewrites `Cargo.toml` and `apps/desktop/src-tauri/tauri.conf.json` to
   the new version (parser-based, no sed).
4. Commits the version bump on `next`.
5. Tags `vX.Y.Z-beta.N` on the new commit.
6. Pushes `next` + the tag.

Example output:

```
[release-promote] branch: next  HEAD: a1b2c3d
[release-promote] current version: 1.9.0-alpha.3
[release-promote] next version:    1.9.0-beta.1
[release-promote] writing Cargo.toml + tauri.conf.json
[release-promote] committing version bump
[release-promote] tagging v1.9.0-beta.1
[release-promote] pushing next + tag to origin
[release-promote] done. CI will fire release.yml for v1.9.0-beta.1.
```

CI then:

- `validate-tag` confirms the tag is on `next` and not on `main`.
- Build matrix runs (Windows MSI, Linux AppImage, etc.).
- `Update channel manifest on main` checks `main` out, writes
  `release-manifests/tray-beta.json`, commits, pushes. Tauri updater clients
  on `beta` see the new version on next poll.

Flags:

- `--sha <S>` — promote a SHA other than `next` HEAD (must be reachable
  from `next`). Lets you hold back later `next` commits.
- `--n <N>` — force the numeric suffix (otherwise auto-increments).
- `--dry-run` — print the plan, touch nothing.

### Re-tagging the same `next` SHA across alpha → beta → rc

Supported and expected. Same commit, different channels:

```bash
# Day 1 — internal smoke
node scripts/release-promote.mjs prerelease alpha
#   → tags v1.9.0-alpha.1 on next HEAD (commit C)

# Day 3 — open to testers, same code
node scripts/release-promote.mjs prerelease beta --sha C
#   → tags v1.9.0-beta.1 on commit C (which is still on next)

# Day 7 — release candidate, same code
node scripts/release-promote.mjs prerelease rc --sha C
#   → tags v1.9.0-rc.1 on commit C
```

Each tag fires `release.yml` independently and writes a different
channel manifest.

---

## 5. Promote to live

### Before you run

Three sharp edges the script doesn't auto-handle today. Get past them
in this order:

1. **Stash uncommitted work.** `git merge --ff-only` (inside the script)
   refuses if any working-tree file would be overwritten, even files
   the merge doesn't touch. Stash first:

   ```bash
   git status                                    # confirm what's dirty
   git stash push -u -m "pre-live-promotion"     # -u also stashes untracked
   ```

   Pop it back after the script succeeds: `git stash pop`.

2. **Make sure local `main` has the latest promotion script.** Node
   loads `scripts/release-promote.mjs` at process start from whatever
   is currently on disk in your local checkout. If you ran the script
   from a `main` that's behind `next` (e.g. you haven't yet promoted a
   `next` that contains a fix to the script itself), Node will run the
   OLD version end-to-end even though the merge during the run pulls
   in a newer script. Symptom: the script tries to `git commit` a
   no-op bump and fails with `nothing to commit, working tree clean`.

   Workaround when you suspect this: pull the latest script from
   `next` onto your local main before starting:

   ```bash
   git fetch origin
   git checkout origin/next -- scripts/release-promote.mjs
   node scripts/release-promote.mjs live
   ```

   (The script-from-next overwrites the working tree's copy, then Node
   loads that newer copy at invocation. The checkout doesn't create a
   commit; the live script's own merge step replaces it with the same
   content via the fast-forward.)

3. **Confirm `main` is a strict ancestor of `next`.** Every alpha tag
   creates a `release-manifests/tray-alpha.json` commit on `main` (via
   `release.yml`'s `Update channel manifest` step) that doesn't get
   back-merged to `next`. After several alphas, `main` is ahead of
   `next` and the live promotion's fast-forward refuses with
   `origin/main is not an ancestor of <next-sha>; fast-forward not
   possible`. Restore Invariant #1 by rebasing `next` onto `main`:

   ```bash
   git fetch origin
   git checkout -b _tmp_rebase origin/next
   git rebase origin/main          # should be conflict-free — only
                                   # release-manifests/*.json differs
   git push --force-with-lease origin _tmp_rebase:next
   git checkout main
   git branch -D _tmp_rebase
   ```

   (Force-pushing `next` is acceptable — it's the integration branch
   and you're the only writer in normal flow. `--force-with-lease`
   prevents clobbering anyone else's push you didn't fetch.)

### Run it

When an RC is ready:

```bash
git fetch origin
node scripts/release-promote.mjs live
```

What it does:

1. Verifies `next` HEAD is ahead of `main` and reachable from `next`.
2. Fast-forwards `main` to `next` HEAD (no merge commit).
3. Bumps `Cargo.toml` + `tauri.conf.json` to the bare semver (strips any
   `-rc.N` suffix → `1.9.0`).
4. Commits the bump on `main`.
5. Tags `v1.9.0` on the new commit.
6. Pushes `main` + tag.

CI:

- `validate-tag` confirms the tag is on `main`.
- Build matrix runs.
- `Update channel manifest on main` writes `release-manifests/tray-live.json`.

### Partial promotion

Holding back later `next` commits? Promote an earlier SHA:

```bash
node scripts/release-promote.mjs live --sha 9f8e7d6
```

The SHA must be reachable from `origin/next`. `next` is **not** reset —
anything past `9f8e7d6` stays pre-release and rides the next promotion.

### After promotion

`main` and `next` are now equal. New feature work on `next` will diverge
forward; the next promotion will fast-forward `main` again.

---

## 6. CI-driven promotion (the `Promote release` workflow)

`.github/workflows/promote.yml` wraps the local script so promotions can
run without a checked-out repo. Two trigger paths:

- **Manual** — `workflow_dispatch` with channel/sha/n/dry_run inputs.
  Available channels: `alpha`, `beta`, `rc`, `live`.
- **Automatic** — every push to `next` auto-fires an alpha. The bump
  commit itself is skipped via the `chore: bump to v...` message check,
  so the workflow can't infinite-loop on itself.

### 6.1. One-time setup (do this before the first run)

#### a) Create a Personal Access Token

The default `GITHUB_TOKEN` cannot be used here: tags pushed by
`GITHUB_TOKEN` do NOT trigger downstream workflows (anti-recursion
guard), so `release.yml` and `release-images.yml` would never fire.

Create a **fine-grained PAT** scoped to this repo:

1. https://github.com/settings/personal-access-tokens/new
2. Owner: `TheCodeSaiyan`, repository access: only `StarStats`.
3. Repository permissions:
   - **Contents: Read and write** (push commits + tags)
   - **Metadata: Read-only** (always required)
4. Expiration: 90-365 days (rotate before it expires).
5. Save the token value somewhere safe — GitHub only shows it once.

Add the token as a repo secret:

```bash
gh secret set RELEASE_PROMOTE_PAT --repo TheCodeSaiyan/StarStats-Platform --body '<paste-token-here>'
```

#### b) Configure the `production-release` Environment

Live promotion requires reviewer approval via a GitHub Environment.

1. https://github.com/TheCodeSaiyan/StarStats-Platform/settings/environments
2. **New environment** → name it `production-release`.
3. Under **Deployment protection rules**, enable **Required reviewers**
   and add yourself (and anyone else who can approve a live release).
4. Save.

Once configured, any `live` dispatch will pause until a reviewer
approves, and the workflow run page shows the dry-run version preview
above the approval button.

### 6.2. Manual dispatch

From the GitHub UI: **Actions → Promote release → Run workflow**.

From the CLI:

```bash
# Ship a beta from next HEAD
gh workflow run promote.yml --field channel=beta

# Ship an RC from a specific next commit, with a forced N
gh workflow run promote.yml \
  --field channel=rc \
  --field sha=abc1234 \
  --field n=2

# Preview a live promotion without pushing
gh workflow run promote.yml \
  --field channel=live \
  --field dry_run=true
```

Inputs:

| Input               | Type     | Required | Notes                                                                                       |
|---------------------|----------|----------|---------------------------------------------------------------------------------------------|
| `channel`           | choice   | yes      | `alpha`, `beta`, `rc`, `live`                                                               |
| `sha`               | string   | no       | Target SHA on `next`. Defaults to `next` HEAD.                                              |
| `n`                 | string   | no       | Explicit pre-release N. Must advance forward.                                               |
| `dry_run`           | boolean  | no       | Prints actions without pushing (default: `false`).                                          |
| `roadmap_item_slug` | string   | no       | Explicit slug for the tag annotation. Overrides auto-discovery from merged PR labels (§12). |

`channel=live` runs both a dry-run preview and the real promotion,
gated by the `production-release` Environment between them.

The `roadmap_item_slug` input is usually unnecessary because the
release-promote script auto-discovers the slug from `roadmap/<slug>`
labels on PRs merged since the previous track tag — see **§12 Roadmap
tracking pipeline** for the full mechanism. Pass `roadmap_item_slug`
when you want to override auto-discovery (e.g., bulk releases that
ship multiple roadmap items, or releases where the labels weren't
applied at PR-create time).

### 6.3. Automatic alpha cadence

Every push to `next` fires the workflow, which:

1. Checks the head commit's message. If it starts with
   `chore: bump to v`, the run is skipped (recursion guard — that's
   the workflow's own bump commit landing).
2. Otherwise runs `node scripts/release-promote.mjs prerelease alpha`
   against `next` HEAD.

You don't need to do anything to opt in. To **temporarily disable**
the auto-alpha (e.g. during a noisy refactor that's churning `next`),
the cleanest option is to push commits with `[skip ci]` in the message
— but that also skips `ci.yml`, which you probably don't want. A
better option is to use the **disable workflow** button at
**Actions → Promote release → ⋯ → Disable workflow**, then re-enable
when you're ready.

### 6.4. Concurrency

All `next`-targeting promotions share the `promote-next` concurrency
group, so a manual `beta` dispatch can't race the auto-alpha (both
would bump `Cargo.toml` and one would lose). Runs queue in arrival
order; in-flight runs are NOT cancelled. The `live` dispatch uses a
separate `promote-live` group since it operates on `main`.

### 6.5. Troubleshooting the workflow

**"Error: Resource not accessible by integration" on `git push`.**
The PAT secret is missing or doesn't have `contents: write`. Re-check
the PAT scope and that `RELEASE_PROMOTE_PAT` is set on the repo
(`gh secret list --repo TheCodeSaiyan/StarStats-Platform`).

**Auto-alpha didn't fire on a push to `next`.** Check the head commit
message — anything starting with `chore: bump to v` is intentionally
skipped. Also check **Actions → Promote release** is not Disabled.

**Live dispatch hangs.** It's waiting for Environment approval at
**Actions → Promote release → \<run\> → Review deployments**. Approve
or reject there.

**Tag pushed but `release.yml` didn't trigger.** The PAT either
expired or got revoked, and the tag push fell back to `GITHUB_TOKEN`
(no cascade trigger). Re-run by deleting the tag remotely + re-running
the workflow, OR push the tag manually with a still-valid PAT.

---

## 7. Hotfix a live bug

Hotfixes are the ONLY path where a commit reaches `main` without going
through `next`.

```bash
git fetch origin
git switch -c hotfix/v1.9.1-fix-foo origin/main

# ... fix, commit, push ...

gh pr create --base main --head hotfix/v1.9.1-fix-foo \
  --title "fix: …" \
  --body "Hotfix for v1.9.0 …"

# After PR merge, tag from main:
git switch main
git pull --ff-only origin main
git tag v1.9.1
git push origin v1.9.1

# CRITICAL: back-merge main → next so next contains the hotfix.
node scripts/release-promote.mjs hotfix-finish
```

`hotfix-finish` merges `main` into `next` (regular merge commit; not a
rebase, because `next` already has divergent history). This restores
**Invariant #1**.

If you skip `hotfix-finish`, the next pre-release you cut from `next`
will be missing the hotfix. The auto-updater will happily serve a beta
that regresses the bug you just fixed on live. `validate-tag` does NOT
catch this — the failing signal is your QA on the next pre-release.

---

## 8. Sibling worktrees

If you have sibling clones (e.g.
`../StarStats-feature-foo`), they currently
track `origin/main`. After the migration:

```bash
cd ../StarStats-feature-foo
git fetch origin
git switch next   # now exists
```

Update any local in-flight feature branches that were based on `main`
to retarget `next`:

```bash
git switch feat/widget-x
git rebase --onto origin/next origin/main
git push --force-with-lease
gh pr edit <NUM> --base next
```

In-flight dependabot PRs targeting `main` can merge through as-is; new
dependabot PRs target `next` via `.github/dependabot.yml`.

---

## 9. Branch protection (manual, optional)

Branch protection is not enforced server-side today; CI guards
(`validate-tag`, `ci.yml` on both branches) do the real work. If you
want symmetric server-side rules on `main` and `next`, run:

```bash
OWNER=TheCodeSaiyan
REPO=StarStats

# Capture current main protection as the source of truth
gh api "/repos/$OWNER/$REPO/branches/main/protection" \
  > /tmp/main-protection.json

# Apply identical rules to next
gh api -X PUT "/repos/$OWNER/$REPO/branches/next/protection" \
  --input /tmp/main-protection.json
```

If you ever change one branch's protection, re-run the snippet for the
other or the channels drift apart.

---

## 10. Troubleshooting

### `validate-tag` failed: "pre-release tag $TAG is on main"

You cut a `-alpha`/`-beta`/`-rc` tag on a commit that's already on
`main`. The tag is nonsensical (it's already live). Options:

- Delete the tag (`git tag -d X; git push --delete origin X`) and tag
  the correct `next`-only SHA.
- If you meant to ship to live, retag as a bare `vX.Y.Z`.

### `validate-tag` failed: "pre-release tag $TAG not reachable from next"

The tag is on a SHA that isn't on `next`. Usually means you tagged a
local branch you forgot to push, or tagged `main` by accident. Move the
tag:

```bash
git tag -d vX.Y.Z-beta.1
git push --delete origin vX.Y.Z-beta.1
git switch next
git pull --ff-only origin next
git tag vX.Y.Z-beta.1
git push origin vX.Y.Z-beta.1
```

### `validate-tag` failed: "live tag $TAG not on main; promote next → main first"

You cut a bare `vX.Y.Z` tag from `next` without promoting. Delete the
tag, run `release-promote.mjs live`, let it tag from `main`.

### Manifest script failed: "Cargo.toml version doesn't match tag"

`Cargo.toml` and `tauri.conf.json` must match the tag's bare semver
(strip suffix). Caused by:

- Tagging without running `release-promote.mjs` (which does the bump).
- Editing one of the version files by hand and forgetting the other.

Fix:

```bash
git tag -d vX.Y.Z-beta.1
git push --delete origin vX.Y.Z-beta.1
# Re-run the promotion script which bumps both files in lockstep.
node scripts/release-promote.mjs prerelease beta
```

### `release-promote.mjs live` refuses: "next not ahead of main"

`main` is already at or past `next`. Either nothing to promote, or a
hotfix landed on `main` and `hotfix-finish` wasn't run. Check:

```bash
git fetch origin
git log --oneline origin/main..origin/next   # should show commits
git log --oneline origin/next..origin/main   # if non-empty, run hotfix-finish first
```

### `git merge --ff-only ...` errored: "Your local changes to the following files would be overwritten by merge"

Mid-promotion, the script's internal `git merge --ff-only` refuses
because your working tree has uncommitted changes (even to files the
merge doesn't touch). Stash and retry:

```bash
git stash push -u -m "live-recover"
node scripts/release-promote.mjs live
git stash pop
```

If the script left `main` partially advanced (e.g. the merge ran but
the bump-commit step failed), see "`nothing to commit, working tree
clean` mid-promotion" below — you may need to finish manually.

### `release-promote.mjs live` errored: "origin/main is not an ancestor of <next-sha>; fast-forward not possible"

`main` has commits `next` doesn't. The usual cause is the
`release-manifests/tray-<channel>.json` auto-commits that `release.yml`
lands on `main` after every alpha/beta/rc tag — those aren't
back-merged to `next` automatically. See §5 step 3 for the rebase
recipe.

### `nothing to commit, working tree clean` mid-promotion

The merge step succeeded (your local `main` advanced to the `next`
SHA) but the bump-commit step refuses because Cargo.toml +
tauri.conf.json already match the target version on the new main.

This is correct behaviour on the current script — there's nothing to
commit when the bare semver is unchanged. But the OLD pre-fix script
(any local `main` checkout that hasn't yet received the fix in
[#77](https://github.com/TheCodeSaiyan/StarStats-Platform/pull/77) and successors)
always tries to commit and dies here.

Recovery without rerunning:

```bash
# Confirm main is at the target SHA already
git log --oneline -3

# Tag manually and push (the bump commit isn't needed)
git tag vX.Y.Z
git push origin main          # may need merge with origin/main first
                              # if alpha manifest commits landed during the run
git push origin vX.Y.Z

# If push origin main was rejected as non-ff:
git fetch origin
git merge origin/main --no-ff -m "merge origin/main into main (manifest catch-up)"
git push origin main
```

The pushed tag fires `release.yml` which builds + publishes the live
release and commits `live.json` to `main`. If the tag already fired
but failed earlier (e.g. main wasn't ready), re-run the failed runs:

```bash
gh run list --branch vX.Y.Z --json databaseId,workflowName,conclusion \
  --jq '.[] | select(.conclusion=="failure") | "\(.databaseId) \(.workflowName)"'
gh run rerun <run-id>
```

### `release-promote.mjs hotfix-finish` reports merge conflicts

`main` and `next` have diverged in a file the hotfix touched. Resolve
the conflicts locally, commit the merge, then push `next`:

```bash
git switch next
git pull --ff-only origin next
git merge origin/main   # resolve conflicts
git push origin next
```

### A dependabot PR still targets `main`

It was opened before the dependabot config change. Either merge it
through to `main` (it'll back-merge into `next` on the next promotion or
hotfix-finish), or close it and let dependabot reopen against `next`.

---

## Quick reference (post-tracks-split)

All `release-promote.mjs` subcommands except `hotfix-finish` now take
a `<track>` argument — `tray` or `platform`. The CI workflow_dispatch
form takes a matching `track` input (with an extra `both` option that
fans out to both tracks).

| Action                            | Command                                                            |
|-----------------------------------|--------------------------------------------------------------------|
| Open feature PR                   | `gh pr create --base next`                                         |
| Cut tray alpha                    | `node scripts/release-promote.mjs prerelease tray alpha`           |
| Cut platform alpha                | `node scripts/release-promote.mjs prerelease platform alpha`       |
| Cut tray beta                     | `node scripts/release-promote.mjs prerelease tray beta`            |
| Cut platform beta                 | `node scripts/release-promote.mjs prerelease platform beta`        |
| Cut tray rc                       | `node scripts/release-promote.mjs prerelease tray rc`              |
| Cut platform rc                   | `node scripts/release-promote.mjs prerelease platform rc`          |
| Retag SHA on next channel (tray)  | `node scripts/release-promote.mjs prerelease tray beta --sha S`    |
| Promote tray → live               | `node scripts/release-promote.mjs live tray`                       |
| Promote platform → live           | `node scripts/release-promote.mjs live platform`                   |
| Partial promote (tray)            | `node scripts/release-promote.mjs live tray --sha S`               |
| After hotfix on main              | `node scripts/release-promote.mjs hotfix-finish`                   |
| Dry-run any of the above          | append `--dry-run`                                                 |
| Cut a channel via CI (one track)  | `gh workflow run promote.yml -f track=tray -f channel=beta`        |
| Cut a channel via CI (both)       | `gh workflow run promote.yml -f track=both -f channel=beta`        |
| Auto-alpha cadence                | every push to `next` — per-track via paths-filter (see §6.3)       |

### Post-split addendum (read this first)

Three behaviour changes from the pre-2026-05-23 model:

1. **Tag schema.** Tray tags now use the `tray-v` prefix; platform tags
   remain bare `vX.Y.Z`. The two tag spaces never overlap (the glob
   `v*` in `release-images.yml` doesn't match `tray-v*`).
2. **Per-track auto-alpha.** A push to `next` no longer always tags an
   alpha. `promote.yml` runs a paths-filter setup job that bumps only
   the tracks whose files actually changed. A docs-only push tags
   neither track. A change touching both tray and platform paths tags
   both.
3. **Channel manifests are `tray-` prefixed.** The Tauri updater reads
   `release-manifests/tray-{alpha,beta,rc,live}.json`. Old in-the-wild
   installers still pointing at `release-manifests/tray-{channel}.json`
   will 404 on their next update check — re-download and re-pair from
   the latest GH release.

Detailed examples for the new subcommands live in the body of this
document with the legacy single-track examples flagged where they
need a track argument added.

---

## 11. Beta staging site (`beta.starstats.app`)

A **web-tier-only** staging deployment for reviewing UI work against
real production data before it ships to `starstats.app`.

```
beta.starstats.app  →  web:beta    ─┐
                                     ├─→  starstats-api  →  live Postgres
starstats.app       →  web:latest  ─┘        (:latest)
```

Both web containers live in the same Compose project and reach the API
over the internal network as `http://starstats-api:8080` — the same
hostname production uses. "Beta hitting live data" is that one line.

### What it is, and what it deliberately is not

* **It is not a separate environment.** There is no `api:beta`, no beta
  database, no beta SpiceDB. The beta web container talks to the same
  `starstats-api` container every user talks to. Anything you do on
  beta — creating a share, revoking a device, resolving a report —
  happens to production data, for real, and lands in the production
  audit chain. Treat it as production with a different skin.
* **It never auto-builds.** `release-images.yml` is push-driven
  (`main` → `:latest`, `next` → `:next`). Beta is not: it builds only
  when an operator dispatches `Release web (beta)`. Staging changing
  under someone who is mid-review is worse than staging being stale.
* **It is public but not indexed.** Anyone with the link can load it.
  `STARSTATS_NOINDEX=1` makes the container serve a blanket
  `Disallow: /` from `/robots.txt` **and** emit
  `<meta name="robots" content="noindex, nofollow">` on every page.
  Both are needed: robots.txt stops well-behaved crawlers fetching,
  the meta tag stops indexing of URLs found by other means. Without
  it, beta is duplicate content competing with the real site.

  **Cloudflare injects a managed `robots.txt` on this zone.** As of
  2026-08-23 `https://starstats.app/robots.txt` returns 200 with a
  "BEGIN Cloudflare Managed content" block (Content-Signal policy plus
  `Disallow: /` for a list of AI crawlers) — the origin serves no
  robots.txt at all today, so that file is entirely Cloudflare's. When
  the origin *does* serve one, Cloudflare appends its managed block
  rather than replacing it, which leaves beta with two `User-agent: *`
  groups: our `Disallow: /` and Cloudflare's `Allow: /`. Crawlers merge
  same-agent groups and resolve ties toward Allow, so **the robots.txt
  half of the noindex may not survive the edge.**

  This is why the meta tag is not redundant belt-and-braces — on this
  zone it is the load-bearing half, and it is also the stronger signal:
  robots.txt only prevents *crawling*, and a disallowed URL can still
  be indexed from inbound links, whereas `noindex` actually deindexes.
  Cloudflare does not rewrite HTML bodies, so the meta tag arrives
  intact. **Verify after the first beta deploy** (see below); if the
  managed block does win, disable the managed robots.txt for the
  `beta.starstats.app` hostname in the Cloudflare dashboard rather than
  weakening the app-side rule.

### Shipping a build to beta

```bash
# Build the current redesign branch and point :beta at it.
gh workflow run release-web-beta.yml -f ref=feat/redesign

# Build a pinned image without moving :beta (rehearsal / rollback prep).
gh workflow run release-web-beta.yml -f ref=feat/redesign   -f move_floating_tag=false
```

Each run publishes:

| Tag | Moves? | Purpose |
| --- | --- | --- |
| `web:beta` | on every run unless `move_floating_tag=false` | what the container pulls |
| `web:beta-<sha7>` | never | immutable; the rollback target |

Then redeploy the `starstats-web-beta` service in Komodo so it re-pulls
`:beta`.

**Rollback** is a repoint, not a rebuild — every past build is still
addressable:

```bash
docker buildx imagetools create   -t registry-direct.tatux.in/starstats/web:beta   registry-direct.tatux.in/starstats/web:beta-<sha7>
```

### Container definition

The service is **`starstats-web-beta`** in
`home-servers-build:compose/starstats/compose.yml` — that file is the
source of truth, not this document. It is the production `starstats-web`
service with one image tag and four env values changed:

| Setting | `starstats-web` | `starstats-web-beta` |
| --- | --- | --- |
| `image` | `registry.tatux.in/starstats/web:latest` | `registry.tatux.in/starstats/web:beta` |
| `ipv4_address` | `192.168.90.182` | `192.168.90.187` |
| `STARSTATS_API_URL` | `http://starstats-api:8080` | `http://starstats-api:8080` (same — live) |
| `STARSTATS_SITE_URL` | `https://starstats.app` | `https://beta.starstats.app` |
| `STARSTATS_NOINDEX` | unset | `"1"` |
| `OTEL_SERVICE_NAME` | `starstats-web` | `starstats-web-beta` |
| Traefik rule | `starstats.app` \|\| `www.` \|\| `stats.$DOMAINNAME` | `beta.starstats.app` |

`NEXT_SERVER_ACTIONS_ENCRYPTION_KEY` is the same value as production and
must be — the `:beta` image is built with the same build-arg secret, and
build value and runtime value have to match or every form submit fails
with `UnrecognizedActionError`. It only pins the content-addressing of
server-action IDs; it shares no trust with the session cookie.

**Registry hosts differ by direction, deliberately.** CI *pushes* to
`registry-direct.tatux.in` (DNS-only, bypasses the Cloudflare proxy that
throttled large layer transfers). The stack *pulls* from
`registry.tatux.in`. Same Distribution backend, so `:beta` pushed by the
workflow is the `:beta` the host pulls.

**Ordering constraint — this bites once.** `registry.tatux.in/starstats/web:beta`
must exist *before* the compose change reaches the host. `pull_policy: always`
on a missing tag fails the pull, and a failed pull fails the whole stack
deploy — which would block a production redeploy of `starstats-web` too.
Dispatch the build first, confirm the tag, then commit the compose file.

**Deploy cadence.** The `starstats` stack runs `poll_for_updates` +
`auto_update`, with a Komodo Action polling every 5 minutes. So beta rolls
automatically within ~5 min of a build — but only ever a build *you*
dispatched, since no branch push produces a `:beta` image. To make the
deploy manual as well, add `"starstats-web-beta"` to `ignore_services`
on the starstats stack in `komodo/resources.toml` and run
`docker compose up -d starstats-web-beta` by hand.

`OTEL_SERVICE_NAME` and `deployment.environment=beta` are deliberately
distinct so beta traces and errors are separable from production ones in
Tempo/GlitchTip — without them, a redesign bug and a live bug look
identical in the dashboards.

**DNS + certificate.** `beta.starstats.app` needs a DNS record in
Cloudflare pointing at the same origin as `starstats.app`. The
certificate is automatic — the router uses the `dns-cloudflare`
certresolver, same as every other host in the stack.

### Verifying a beta deploy

Per the deploy-verification rule, `healthz` proves nothing about the web
tier — check the CSS content hash, which is content-addressed:

```bash
curl -s https://beta.starstats.app/ | grep -o '/_next/static/css/[^"]*'
curl -s https://starstats.app/     | grep -o '/_next/static/css/[^"]*'
# Different hashes ⇒ beta really is running different code.

# Expect our `Disallow: /`. If the only User-agent group present is the
# Cloudflare Managed one with `Allow: /`, the edge overrode the origin —
# see the Cloudflare note above. Compare against the origin's own view.
curl -s https://beta.starstats.app/robots.txt

# The load-bearing check on this zone — Cloudflare does not touch HTML.
curl -s https://beta.starstats.app/ | grep -o '<meta name="robots"[^>]*>'
# expect: <meta name="robots" content="noindex, nofollow, nocache"/>
```

### The `:<sha>` trap

`release-web-beta.yml` publishes `beta` and `beta-<sha7>` and
**must never publish a bare `web:<full-sha>` tag.**

`release-images.yml` preflights `web:<full-sha>`; when it finds one it
*retags that manifest* instead of rebuilding (the one-build-per-SHA
optimisation). If the beta workflow published `:<full-sha>` for a
redesign commit, a later live promote of that same commit would find
it, retag the staging build as `:latest`, and ship it to production
without building anything. The `tags:` list in that workflow carries a
comment saying so; leave it there.

---

## 12. Roadmap tracking pipeline

Shipped 2026-05-27 → 2026-05-29 (PRs #112, #113, #125, #126, #128,
#129, #130, #133). Closes the loop between "this PR ships feature X"
and "the tray 'What's new' panel publishes when X reaches a channel"
with zero operator typing between feature start and tray notification.

### 11.1. The six-step chain

1. **Feature start** — `roadmap-tracking` skill
   (the release tooling) fires post-brainstorming,
   pre-TDD. Asks "link existing item / create new / skip tracking" and
   stashes the decision at `.local/session-roadmap-slug`.

2. **PR create** — `pr-roadmap-link` skill
   (the release tooling) fires when about to
   `gh pr create --base next`. Plants two attribution marks per spec §3.3:
   - **`roadmap/<slug>` label on the PR** — **canonical**. Auto-discovery
     reads this.
   - `Roadmap-Item: <slug>` trailer in the PR body — human-readable
     only. NOT read by any automation; future-you reading the merged
     PR can see the slug without clicking through to labels.

3. **Release time** — `scripts/release-promote.mjs` auto-discovers the
   slug from `roadmap/*` labels on PRs merged in the range `(prev_tag,
   target_sha]`. Resolution priority:
   1. `--roadmap-item-slug X` explicit flag wins.
   2. `--no-auto-slug` flag → skip discovery, tag unannotated.
   3. Auto-discover (default): 0 slugs → no annotation; 1 → use it;
      2+ → refuse, demand explicit choice (so multi-slug releases
      can't silently mis-attribute).

4. **Tag annotation** — release-promote writes
   `Roadmap-Item: <slug>` into the annotated tag body. The release
   workflow's "Resolve roadmap item slug" step parses it via
   `git tag -l --format='%(contents)'`.

5. **CI emit** — `scripts/roadmap-emit-event.mjs` POSTs a signed CI
   event (HMAC-SHA256 over `v1.<ts>.<body>`) to
   `POST /v1/internal/roadmap/events`. Server creates a draft
   changelog entry on the channel's first-time-Shipped transition.

6. **CI auto-publish** — `scripts/auto-publish-changelog.mjs` POSTs
   another signed request to `POST /v1/internal/roadmap/changelog/publish`
   (the HMAC endpoint shipped in #128). Server publishes the draft,
   fans out subscriber notifications, fires the Discord webhook, and
   the tray "What's new" card surfaces on next refresh.

Steps 1–4 are operator-facing; 5–6 happen in CI without intervention.

### 11.2. When to skip tracking

The `pr-roadmap-link` skill silently skips itself when:

- Branch name starts with `docs/`, `chore/`, `style/`, `refactor/`,
  `release/`, or `revert/`.
- All commits use Conventional Commits prefixes that don't ship
  user-visible behavior (`chore:`, `docs:`, `style:`, `test:`, `ci:`).
- You say "no roadmap" / "skip tracking" / "untracked" in the same turn.
- PR body already carries a `Roadmap-Item:` trailer (idempotent re-runs).

A skipped PR contributes no slug to the eventual release's discovery
window. If the release contains only skipped PRs, you'll see
`[track] slug: (none) — N PR(s) since <prev_tag>, none with roadmap/* labels`
in the promote output and the tag will be unannotated. That's the
right outcome for infrastructure releases.

### 11.3. Manual overrides

```bash
# Override auto-discovery with an explicit slug
node scripts/release-promote.mjs live tray --roadmap-item-slug my-feature

# Skip discovery entirely (multi-item release, or unlabeled work)
node scripts/release-promote.mjs live tray --no-auto-slug
```

The CI workflow has the equivalent input
(`workflow_dispatch.roadmap_item_slug`) — see §6.2.

### 11.4. Auto-discovery details

`discoverSlugFromMergedPrs` in `scripts/release-promote.mjs`:

1. Finds the previous track tag via `previousTrackTagBelow(tagList,
   track, currentVersion)` — channel-agnostic but track-scoped.
2. Gets the prev tag's commit date for a coarse `merged:>{date}`
   filter on `gh pr list`.
3. Calls `gh pr list --state merged --base next --search merged:>{date}
   --json number,mergeCommit,labels,title`.
4. Narrows by exact ancestry: keeps PRs whose `mergeCommit.oid` is
   reachable from `targetSha` AND NOT reachable from `prevTag`.
5. Extracts unique `roadmap/<slug>` label suffixes via
   `parseSlugsFromPrLabels`.

Failure-tolerant: any gh / git error returns null and the release
proceeds with no annotation. Network issues never block a release.

### 11.5. Server-side dependencies

- HMAC key: `ROADMAP_CI_EVENT_HMAC_KEY` (shared between emit + publish
  endpoints — one secret, one auth scheme).
- API base: `STARSTATS_API_URL` (shared with the existing admin-CLI
  publish script; one secret per environment, paths appended in code).
- Webhook subscription: the org-level GitHub webhook MUST be
  subscribed to BOTH `Projects v2 item` AND `Issues` events. Without
  the Issues subscription, label changes on Issues don't propagate to
  the server until the 5-min reconciler tick (see PR #112).

### 11.6. Operator quick reference

| Symptom | Where to look |
|---|---|
| Release shipped, no changelog drafted | Tag annotation missing `Roadmap-Item:`? Check `git tag -l --format='%(contents)' <tag>`. |
| Release shipped, draft created, not published | Auto-publish job log. `ROADMAP_ITEM_SLUG: ` empty → slug not in tag annotation. `[auto-publish] no-op: STARSTATS_API_URL not set` → secret missing. |
| Auto-discovery picked wrong slug | Multiple labeled PRs in range. Pass `--roadmap-item-slug X` explicitly. |
| Auto-discovery picked NO slug | Check `gh pr list --base next --search "merged:>{date}" --json labels` against the prev-tag's date. If labels are missing on PRs, the `pr-roadmap-link` skill didn't fire (or skipped per §12.2). |
| Label exists on PR but discovery still misses | Verify `gh` CLI is on PATH for the release-promote script. Auto-discovery skips silently on `gh` errors. |
