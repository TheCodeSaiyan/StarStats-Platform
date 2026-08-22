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

## The federated credential subject — settled 2026-08-22

`tray-v0.1.3-alpha.8` was the first release to run this path. It **failed**,
and the failure is now understood. Read this before touching the credential.

### What the token actually presents

```
repo:TheCodeSaiyan@287788021/StarStats-Platform@1339014359:ref:refs/tags/tray-v0.1.3-alpha.8
```

Two things this settles:

1. **The `@<id>` immutable format is correct.** This repo does emit owner-id /
   repo-id subjects, so `TheCodeSaiyan@287788021/StarStats-Platform@1339014359`
   is right and survives the rename from `TheCodeSaiyan/StarStats`. No action
   needed on that half.
2. **The tag segment is the literal tag**, not a pattern. It changes on every
   release, which is the whole difficulty.

Why this repo emits the immutable format at all: GitHub applies it
automatically to repositories that are created, **renamed**, or transferred
from 2026-07-15 onward. This repo was renamed `StarStats` →
`StarStats-Platform`, so it was opted in without anyone choosing it.

### Why it failed

The credential was created through the portal's "Based on Selection" flow with
`...:ref:refs/tags/tray-v*`. That flow produces a **classic** federated
credential, and a classic credential compares `sub` **byte-for-byte**. The `*`
is matched as a literal asterisk, so it matches nothing, ever.

```
AADSTS700213: No matching federated identity record found for presented
assertion subject 'repo:TheCodeSaiyan@287788021/***@1339014359:ref:refs/tags/tray-v0.1.3-alpha.8'
```

(The repo name shows as `***` because GitHub redacts any string equal to a
secret value, and `ARTIFACT_SIGNING_PROFILE` is `StarStats-Platform` — that is
the real certificate profile name, confirmed, not a paste error. Expect this
redaction in every signing log; it is cosmetic.)

The error message is the diagnostic: it quotes the subject it received. Match
the credential to that string and the problem is over.

### The exact credential to create

Verified against Microsoft Learn on 2026-08-22 (docs revised 2026-08-14), not
from memory. Sources are listed at the end of this section.

Identifiers for this repo, read out of the OIDC token in the alpha.8 failure:

| | |
|---|---|
| `repository_owner_id` | `287788021` |
| `repository_id` | `1339014359` |
| Issuer | `https://token.actions.githubusercontent.com` |
| Audience | `api://AzureADTokenExchange` |

**GitHub flexible credentials have a mandatory-claims rule.** The expression
must match `sub` **and** one or both of `repository_id` /
`repository_owner_id`. This is not optional and applies regardless of whether
`sub` is name-based or immutable. Omitting them produces:

```
Failed to add federated credential. Error detail: The
FederatedIdentityCredential.ClaimsMatchingExpression.Value is invalid.
Rule exception: Expression configured for issuer
'https://token.actions.githubusercontent.com' either lacks all required
claims or contains unallowed claims.
```

That is a rule change that arrived with GitHub's immutable subjects on
2026-07-15 and was missing from the portal's own help text for a while — it is
not a service outage, and retrying or switching app registrations will not
help. Credentials created *before* the change are grandfathered, which is why
an existing one elsewhere can keep working while a new one refuses to save.

The credential:

```json
{
  "name": "starstats-tray-release-tags",
  "issuer": "https://token.actions.githubusercontent.com",
  "audiences": ["api://AzureADTokenExchange"],
  "claimsMatchingExpression": {
    "value": "claims['sub'] matches 'repo:TheCodeSaiyan@287788021/StarStats-Platform@1339014359:ref:refs/tags/tray-v*' and claims['repository_id'] eq '1339014359' and claims['repository_owner_id'] eq '287788021'",
    "languageVersion": 1
  }
}
```

`languageVersion` is always `1`. `subject` must be absent/null —
`claimsMatchingExpression` and `subject` are mutually exclusive.

### Creating it in the Azure portal

**Pick "Other issuer", not the GitHub scenario.** The GitHub Actions scenario
is the "Based on Selection" wizard that produced the broken credential in the
first place: it writes a fixed `subject` and has no expression field, so the
`*` becomes a literal.

1. **Microsoft Entra ID** → **App registrations** → the app whose client ID is
   in `AZURE_CLIENT_ID`.
2. **Certificates & secrets** → **Federated credentials** tab →
   **+ Add credential**.
3. **Federated credential scenario** → **Other issuer**.
4. **Issuer**: `https://token.actions.githubusercontent.com`
5. **Value**: the `claimsMatchingExpression` string above (the expression only,
   not the whole JSON).
6. **Add**.

### Creating it via Graph

Portal and Microsoft Graph are the **only** two supported paths. `az ad app
federated-credential`, `Az` PowerShell, and the Terraform provider have no
flexible-credential support — they error on create *and* on read, so a
credential created in the portal will look broken or invisible to them. `az
rest` works because it is a raw Graph call:

```bash
# objectId is the app registration's OBJECT id, not the client id:
#   az ad app show --id <AZURE_CLIENT_ID> --query id -o tsv
az rest --method post \
  --url https://graph.microsoft.com/beta/applications/<objectId>/federatedIdentityCredentials \
  --body @fic.json
```

Or Graph Explorer: `POST https://graph.microsoft.com/beta/applications/{objectId}/federatedIdentityCredentials`
with the JSON above as the body.

### Then delete the broken one

The classic credential with the literal `refs/tags/tray-v*` subject cannot be
converted — a classic credential has a `subject`, a flexible one has an
expression, and they are mutually exclusive. Leave it in place until the new
one is proven, then remove it so a dead trust isn't sitting on the app (the
per-app limit is 20 credentials).

### What this expression does and does not cover

Covers every `tray-v*` tag push, which is the only trigger that runs
`release.yml`'s signing path in practice.

**Does not cover `workflow_dispatch`.** A manual run from a branch presents
`...:ref:refs/heads/next`, which no tag expression matches. The language has
`matches`, `eq` and `and` — there is **no `or`** — so one expression cannot
span both. If manual release runs need to sign, either:

- add a **second** flexible credential for `...:ref:refs/heads/*`, or
- scope to an environment — add `environment: release` to the
  `client-binaries` job in `release.yml` and match
  `...:environment:release`, which then covers tags and dispatch under one
  subject.

Broadening the tag wildcard to `...@1339014359:*` (the shape Microsoft's own
example uses) would cover everything from this repo, including every branch
push. For an identity that can *sign code*, that is a deliberately larger
blast radius than it looks — prefer the tag-scoped expression.

### Status caveats

Flexible federated identity credentials are in **preview**. They work on
**application objects only** — not user-assigned managed identities. This
setup uses an app registration, so that limitation does not bite.

### Sources

- [Flexible federated identity credentials (preview)](https://learn.microsoft.com/en-us/entra/workload-id/workload-identities-flexible-federated-identity-credentials)
- [Set up a flexible federated identity credential (preview)](https://learn.microsoft.com/en-us/entra/workload-id/workload-identities-set-up-flexible-federated-identity-credential)
- [Migrate GitHub Actions federated credentials to immutable subjects](https://learn.microsoft.com/en-us/entra/workload-id/workload-identities-github-immutable-subjects)
- [Immutable subject claims for GitHub Actions OIDC tokens](https://github.blog/changelog/2026-04-23-immutable-subject-claims-for-github-actions-oidc-tokens/)

### The other prerequisite, still unverified

The service principal needs the **Trusted Signing Certificate Profile Signer**
role on the signing account or profile. Nothing has exercised this yet, because
no build has got past `azure/login`. When the credential is fixed, a 403 from
`signtool` — *after* a successful login — means this role is missing. Do not
re-debug the credential when that happens.

### Blast radius while it is broken

A failing `Azure login` fails `Tauri client windows-latest`, which skips
`Publish GitHub Release`, so **no tray release is published at all** — the
Linux build succeeds but ships nowhere. The platform track (`v*` tags,
container images) is unaffected; only `tray-v*` runs `release.yml`.

Deleting `ARTIFACT_SIGNING_ACCOUNT` restores publishing immediately via the
self-signed fallback, and re-adding it re-enables Trusted Signing. One command
each way, no code change:

```bash
gh secret delete ARTIFACT_SIGNING_ACCOUNT --repo TheCodeSaiyan/StarStats-Platform
```

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
   subject out of the `AADSTS700213` message and fix the credential — the
   message quotes exactly what the token presented.
4. Watch `Configure Artifact Signing` — it prints
   `Artifact Signing configured; profile=…` and throws if `signCommand`
   injection failed.
5. Download the artefact and run the `Get-AuthenticodeSignature` check above.
6. Only then promote to live.
