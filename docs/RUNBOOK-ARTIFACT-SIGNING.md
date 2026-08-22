# Runbook — Windows code signing (Azure Trusted Signing)

Covers how the Windows installers get signed, what has to be true for a
release build to succeed, how to verify a signed artefact, and how to fall
back if signing breaks. Written 2026-08-22 when Trusted Signing replaced the
self-signed certificate.

---

## The important thing first: nothing here is a stored credential

There is no signing key, PFX, or client secret anywhere in CI. Authentication
is **OIDC federation** — the release job proves who it is with a token GitHub
mints at run time, and Azure decides whether to trust it.

```
GitHub Actions runner
  │  permissions: id-token: write        (release.yml, client-binaries job)
  │
  ├─(1)─> GitHub OIDC provider          → short-lived JWT
  │        sub = repo:<org>/<repo>:ref:refs/tags/tray-vX.Y.Z
  │        aud = api://AzureADTokenExchange
  │
  ├─(2)─> Entra ID token endpoint       (azure/login@v2)
  │        client_assertion = that JWT
  │        + AZURE_CLIENT_ID, AZURE_TENANT_ID
  │        Entra matches the JWT's `sub` against a FEDERATED IDENTITY
  │        CREDENTIAL on the app registration. Match → Azure access token.
  │
  └─(3)─> https://<region>.codesigning.azure.net   (signtool + dlib)
           account = ARTIFACT_SIGNING_ACCOUNT
           profile = ARTIFACT_SIGNING_PROFILE
           Authorized by Azure RBAC, not by the token alone.
```

So the six repo secrets are **addressing, not authentication**:

| Secret | What it is | Source of truth |
|---|---|---|
| `ARTIFACT_SIGNING_ACCOUNT` | Trusted Signing account name (`starstats-github-signing`) | Azure portal |
| `ARTIFACT_SIGNING_ENDPOINT` | Regional data-plane URL (`https://neu.codesigning.azure.net`) | 1Password → *StarStats - Azure Artifact Signing Endpoint* |
| `ARTIFACT_SIGNING_PROFILE` | Certificate profile name | 1Password → *StarStats - Azure Artifact Signing Profile* |
| `AZURE_CLIENT_ID` | App registration (client) ID | 1Password → *StarStats - Azure Artifact Application Registration* |
| `AZURE_TENANT_ID` | Entra tenant ID | same item |
| `AZURE_SUBSCRIPTION_ID` | Subscription holding the signing account | same item |

1Password convention for these items: the **username field holds the GitHub
secret name** and the **password field holds its value**, except the
Application Registration item, which uses labelled fields named after the
secrets directly.

Re-uploading all of them in one prompt:

```bash
# signing.env holds op:// references only — never values.
op run --env-file=./signing.env -- bash ./set-signing-secrets.sh
```

Pipe values to `gh secret set` on **stdin**, never `--body` (argv is visible
in `ps`), and never echo them.

---

## The switch

`release.yml` chooses its signing path on one variable:

```yaml
env:
  ARTIFACT_SIGNING_ACCOUNT: ${{ secrets.ARTIFACT_SIGNING_ACCOUNT }}
...
- name: Azure login (Artifact Signing)
  if: matrix.os == 'windows-latest' && env.ARTIFACT_SIGNING_ACCOUNT != ''
- name: Import Windows code-signing cert       # self-signed fallback
  if: matrix.os == 'windows-latest' && env.ARTIFACT_SIGNING_ACCOUNT == ''
```

- **Set** → path A (Trusted Signing). Tauri gets a `bundle.windows.signCommand`
  that shells out to `signtool` with `Azure.CodeSigning.Dlib.dll`. The key
  never exists on the runner.
- **Empty / deleted** → path B, the old self-signed PFX
  (`WINDOWS_CERTIFICATE`). Still present, still works.

**This is also the rollback.** Deleting `ARTIFACT_SIGNING_ACCOUNT` reverts the
next release to self-signed with no code change:

```bash
gh secret delete ARTIFACT_SIGNING_ACCOUNT --repo TheCodeSaiyan/StarStats-Platform
```

There is a deliberate guard against half-configuration — if `ACCOUNT` is set
but `ENDPOINT` or `PROFILE` is empty, the job throws rather than quietly
building unsigned.

---

## ⚠ Unverified: does the federated credential subject actually match?

**This has never completed a real release. Verify it on the first alpha.**

The credential was created with the portal's "Based on Selection" flow and
renders as:

```
repo:TheCodeSaiyan@287788021/StarStats-Platform@1339014359:ref:refs/tags/tray-v*
```

Two things about that string are worth being nervous about:

1. **The `@<id>` suffixes.** That is GitHub's *immutable subject* format,
   which embeds the owner ID and repo ID so the subject survives a rename.
   GitHub only emits it if the repository is opted into that format. The
   default `sub` is plain `repo:TheCodeSaiyan/StarStats-Platform:ref:...`. If
   the repo is not opted in, Entra will be matching a string the token never
   contains.
2. **The trailing `*`.** Classic federated credentials match `sub` *exactly*
   and do not support wildcards; only flexible credentials
   (`claimsMatchingExpression`) do. A literal `*` in a classic subject never
   matches anything.

Both are plausible-and-correct if the portal created a flexible credential
against the immutable format — but neither is verifiable from this repo, and
the failure is invisible until a tag is pushed.

The rename matters here: this repo used to be `TheCodeSaiyan/StarStats` and is
now `TheCodeSaiyan/StarStats-Platform`. `.claude/rules/release-ci.md` and the
`pr-roadmap-link` skill still reference the old name.

### How it fails

`Azure login (Artifact Signing)` fails with `AADSTS70021: No matching
federated identity record found for presented assertion subject`, and the
error message quotes the subject it *did* receive. That quoted string is the
answer — put it in the credential.

### Also check `workflow_dispatch`

A manual run from a branch emits `...:ref:refs/heads/next`, which will not
match a tag-scoped credential no matter how it is written. Dispatch runs will
fail to sign even when tag pushes work. If manual release runs are needed, add
a second credential for the branch ref, or scope the job to an environment
(`repo:<org>/<repo>:environment:release` — needs an `environment:` key on the
`client-binaries` job) so one subject covers both.

### The other Azure-side prerequisite

The service principal needs the **Trusted Signing Certificate Profile Signer**
role on the signing account (or on the specific profile). Without it,
`azure/login` succeeds and `signtool` then fails with a 403 from the data
plane — a confusingly late failure. Check role assignments before blaming the
credential.

---

## Verifying a signed artefact

The certificate subject is:

```
CN=TheCodeSaiyan Ltd, O=TheCodeSaiyan Ltd, L="Eton Wick, Windsor", C=GB
```

```powershell
Get-AuthenticodeSignature .\StarStats_x64-setup.exe |
  Format-List Status, StatusMessage, SignerCertificate
```

`Status` must be `Valid`. Check the signer subject matches above and that the
chain terminates in a Microsoft public root — not a self-signed leaf.

**Do not publish a thumbprint to users.** Trusted Signing issues short-lived
certificates and rotates them continuously, so any thumbprint printed on the
website goes stale within days and would make a genuine download look forged.
The stable identity is the subject name. The user-facing copy on `/downloads`
and `/docs` says exactly this; the constants live in
`apps/web/src/lib/signing.ts` so the two pages cannot drift.

The retired self-signed thumbprint was
`10B7 4595 1DB5 0B9F F046 C39D 5A79 1F65 2EDF E85C` — recorded here only so an
old installer someone still has can be identified. Nothing new will carry it.

---

## SmartScreen is a separate system

A valid CA-issued signature removes "Unknown publisher". It does **not**
immediately remove the SmartScreen "Windows protected your PC" prompt —
reputation accrues per signing identity as installs happen, and a newly issued
identity starts at zero.

Do not tell users the warning is gone. The download page deliberately says the
publisher is now named and verifiable *and* that SmartScreen may still prompt
for a while. Over-promising here is worse than the warning itself: this is the
page that asks people to trust the app with an RSI session cookie.

---

## Order of operations for the first signed release

1. Confirm all six secrets exist:
   `gh secret list --repo TheCodeSaiyan/StarStats-Platform | grep -E 'ARTIFACT|AZURE'`
2. Push an **alpha** tag to `next` first — never validate signing on a live
   promote.
3. Watch the `Azure login (Artifact Signing)` step. If it fails, read the
   subject out of the `AADSTS70021` message and fix the credential.
4. Watch `Configure Artifact Signing` — it prints
   `Artifact Signing configured; profile=…` and throws if `signCommand`
   injection failed.
5. Download the artefact and run the `Get-AuthenticodeSignature` check above.
6. Only then promote to live.
