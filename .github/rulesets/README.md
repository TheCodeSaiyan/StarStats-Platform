# Repository rulesets

`next-main.json` protects `next` and `main`:

- **No deletion and no force push.**
- **The `Commit rules` check must pass** (`.github/workflows/ci.yml`), because
  release notes are generated from commit subjects with no rewrite step.

## What is deliberately not in it, and why

- **Linear history.** `release.yml` merges the live manifest back into `next`,
  and `release-promote.mjs hotfix-finish` merges `main` into `next`. Both are
  merge commits; requiring linear history would break every release.
- **Required pull requests.** `release-promote.mjs` fast-forwards `main` and
  pushes version-bump commits directly.

## Bypass actors

A required status check also blocks a *direct push* of a commit that has not
passed it. Automation pushes such commits, so it bypasses:

| Actor | Why |
|---|---|
| `OrganizationAdmin` (id 1) | `RELEASE_PROMOTE_PAT` belongs to the organisation owner; it pushes bump commits and tags. |
| `Integration` 15368 (GitHub Actions) | `release.yml` commits `release-manifests/*` to `main` and merges them into `next` with `GITHUB_TOKEN`. |

Check both before applying: if the promote token's owner is not an org admin,
add that user as a bypass actor instead, or the next live promote is refused.

## Applying

Applying changes live repository settings, so it is done by the owner, not
by CI:

```sh
gh api -X POST repos/TheCodeSaiyan/StarStats-Platform/rulesets --input .github/rulesets/next-main.json
```

To change it later, `GET` the ruleset id from `repos/{owner}/{repo}/rulesets`
and `PUT` the file to `repos/{owner}/{repo}/rulesets/{id}`. To remove it,
`DELETE` the same path.
