# Repository rules

Release notes are generated from commit subjects with no rewrite step, so a
subject that breaks the rules in `.github/commit-rules.json` ends up in front of
players. This page says what enforces those rules today, and what does not.

## What is enforced

| Where | What | Blocks |
|---|---|---|
| `commit-msg` hook | `scripts/check-commits.mjs --message-file`, installed by `pnpm install` (`scripts/install-hooks.cjs`) | A bad subject at `git commit`, on any machine that has run `pnpm install` |
| CI job **Commit rules** | `scripts/check-commits.mjs --range` over the PR, or over `before..sha` on a push to `next` or `main` | Nothing by itself: it fails the PR visibly, and merges wait for green by convention |
| Ruleset `protect-release-branches` | No deletion, no force push, on `next` and `main` | Rewriting either branch's history, for everyone including admins |

The hook leaves any existing `commit-msg` hook alone and never sets
`core.hooksPath`, so it cannot switch off another tool's hooks.

## What is not enforced, and why

**The Commit rules check is not a required status check.** A PR can still be
merged while it is red. What that costs is a badly worded line in the next
release's notes; nothing breaks.

It is not required because a required check also refuses a *direct push* of
any commit that has not passed it, and two kinds of commit are pushed directly:

- `release.yml` commits `release-manifests/*` to `main` and merges it back into
  `next` after every tray release, pushing with `GITHUB_TOKEN`, which acts as
  GitHub Actions.
- `release-promote.mjs` pushes version bumps and fast-forwards `main`: in CI
  with the promote token, and on a live promote with the operator's own login.
  Both act as the organisation owner.

The owner can be exempted from a repository ruleset. GitHub Actions cannot: a
repository ruleset refuses it as a bypass actor ("must be part of the ruleset
source or owner organization"), and the organisation-level ruleset that would
accept it needs GitHub Team (tried in September 2026 and refused on the Free
plan).

The one alternative on Free is to push the manifest commits with the promote
token instead of `GITHUB_TOKEN`. That is worse than the gap: pushes made with a
personal token start other workflows, and `release-images.yml` builds on every
push to `main` whatever the commit message says, so each tray release would also
rebuild and redeploy the server and web images.

## If the plan changes

On GitHub Team, create an organisation ruleset scoped to `StarStats-Platform`,
targeting `refs/heads/next` and `refs/heads/main`, with one rule: required
status check `Commit rules`, pinned to integration 15368 (GitHub Actions) so no
other app can satisfy it by posting a status with the same name. Bypass actors:
`OrganizationAdmin` (for the promote token, which must belong to an
organisation admin) and `Integration` 15368 (for the manifest commits). Leave
deletion and force push to `protect-release-branches`.

Do not add linear history or required pull requests to either ruleset. Both
release paths above create merge commits or push directly, and either rule
would refuse every release.
