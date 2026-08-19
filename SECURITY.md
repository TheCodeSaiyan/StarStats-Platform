# Security Policy

StarStats is a hobby project run by a single maintainer. We take
security seriously but the response surface is one person, so set
your expectations accordingly. The disclosures below describe how to
report a problem, what's in scope, how to verify release artifacts,
and which dependency advisories we have already triaged and accepted.

## Reporting a vulnerability

**Please do not open a public issue for security problems.**

Preferred channel — **GitHub Security Advisories** (private):
<https://github.com/TheCodeSaiyan/StarStats-Platform/security/advisories/new>.
This goes straight to the maintainer and is invisible until we
publish.

Email channel — **<security@starstats.app>**. Goes to the same
maintainer. The mailbox lives on the project's primary domain, so
domain-MX changes don't affect this address. PGP not yet available;
if you want an encrypted channel, mention it in your first message
and we'll arrange one.

We aim to **acknowledge within 7 days** and to triage within 30.
Critical issues affecting authentication, account takeover,
unintended data disclosure, or the release-signing chain get
prioritised over everything else, including ongoing feature work.

We do not run a paid bug-bounty programme. We are happy to credit
researchers in the corresponding GHSA / release notes if you'd like
the recognition.

## What's in scope

- The Tauri **tray client** (`crates/starstats-client` and
  `apps/tray-ui`), including how it stores the RSI session cookie,
  parses `Game.log`, and handles updater downloads.
- The **API server** (`crates/starstats-server`), including auth,
  device pairing, ingest, query endpoints, the OIDC discovery
  surface (`/.well-known/*`), and the audit log.
- The **web app** (`apps/web`), including login, session cookies,
  email verification, share-link flows, and any code path that
  reaches the API server on the user's behalf.
- The **updater pipeline** end-to-end: per-channel manifests at
  `release-manifests/tray-{alpha,beta,rc,live}.json` on `main`, the Tauri
  updater config in `crates/starstats-client/tauri.conf.json`, the
  signed installers themselves, and the minisign signatures CI
  produces during the release workflow.
- The **WiX / NSIS Windows installers** (`StarStats_<version>_x64-setup.exe`
  and the `.msi`), including elevation behaviour, install-path
  handling, upgrade metadata, and any custom action.
- The **auto-update signature chain**: the minisign keypair used to
  sign updater payloads, the code-signing certificate used for the
  Windows installer, and the chain of trust from a fresh install to
  a verified update.
- Database **migrations and schema** in
  `crates/starstats-server/migrations` — including any path that
  could leak data across users or bypass row-level checks.

## What's out of scope

- **RSI's own services.** The Roberts Space Industries website,
  Spectrum, the launcher, and the game itself are not ours to
  defend. Report vulnerabilities in those to RSI directly.
- **Third-party dependencies with their own security pipelines.**
  Issues in upstream crates / npm packages should be reported to
  the upstream maintainer (or via the Rust Security Response WG /
  GitHub advisory database). If a triaged advisory becomes
  exploitable in *our* codebase, that's in scope and we'll re-open;
  see the accepted-advisories table below.
- **Social-engineering of the maintainer.** Phishing, account-recovery
  abuse, credential stuffing against the maintainer's personal
  accounts, and similar are not paid attack surface; if something
  worked you can let us know but it isn't a vulnerability in
  StarStats.
- **Volumetric DoS / DDoS.** The hosted server runs on a single
  homelab host.
- **Self-XSS** or attacks that require a malicious extension already
  installed in the victim's browser.
- **Bugs in unreleased branches.** Wait until they reach an alpha
  release.

If you find StarStats doing something that *would* trip Easy
Anti-Cheat, that **is** an in-scope security issue (it'd be a bug in
the project's core invariant). The posture itself is documented in
[`EAC-SAFETY.md`](EAC-SAFETY.md).

## Verifying release artifacts

Every tagged release publishes signed installers plus a per-platform
minisign signature in the GitHub release assets. The auto-updater
verifies these signatures automatically; if you'd like to verify a
download by hand:

### Updater minisign public key

```
RWREXTOuz2vAwn392/m6ZqHYEg89ZxiCAh6p6jIjCtAlozSk/mJidZyI
```

This key is also embedded in
`crates/starstats-client/tauri.conf.json` under
`plugins.updater.pubkey` (base64-encoded for Tauri's config layout).
Both copies must match — if they don't, the updater would refuse to
apply an update, and you should refuse to trust the release.

### Verifying a Windows installer

```powershell
# 1. Confirm Authenticode signature (signed with the project code-signing cert):
Get-AuthenticodeSignature .\StarStats_<version>_x64-setup.exe

# 2. Confirm the minisign signature attached to the installer's updater bundle
#    (download the matching .sig from the GitHub release page):
minisign -V -P RWREXTOuz2vAwn392/m6ZqHYEg89ZxiCAh6p6jIjCtAlozSk/mJidZyI `
         -m StarStats_<version>_x64-setup.exe `
         -x StarStats_<version>_x64-setup.exe.sig
```

### Verifying a Linux installer

```bash
# AppImage:
minisign -V -P RWREXTOuz2vAwn392/m6ZqHYEg89ZxiCAh6p6jIjCtAlozSk/mJidZyI \
         -m StarStats_<version>_amd64.AppImage \
         -x StarStats_<version>_amd64.AppImage.sig
```

If a verification step fails, **do not run the installer**. Open a
GitHub Security Advisory or email <security@starstats.app> with the
artifact filename, the release tag, and the exact failure message.

## Security architecture (quick reference)

For deeper detail see [`apps/web/src/app/privacy/page.tsx`](apps/web/src/app/privacy/page.tsx)
§9, which describes what's customer-visible. In summary:

- Passwords: Argon2id.
- Sessions: HttpOnly + Secure cookies, 1-hour TTL.
- API tokens: RS256 JWT signed by an on-disk key generated at first
  boot.
- TOTP secrets: AES-256-GCM-encrypted with a file-based KEK
  (`totp-kek.bin`), fresh nonce per encrypt, never logged.
- Recovery codes: Argon2-hashed; we cannot read them back.
- Magic-link tokens: 15-minute, single-use, atomic
  `UPDATE … RETURNING` redeem so a token can't be replayed.
- Interim 2FA tokens: 5-minute, single-use, useless without the
  matching second factor.
- Anti-enumeration: magic-link start and password-reset start return
  the same response shape regardless of whether the email maps to an
  account, with timing equalised on the miss path.
- RSI session cookie (tray client only): stored in the OS keychain
  (Windows Credential Manager / macOS Keychain / Linux Secret
  Service) — only the same OS user that pasted the cookie can read
  it back.

## Known accepted dependency advisories

The following Dependabot advisories have been triaged and accepted as
**non-exploitable in this codebase**. They remain dismissed in the
GitHub Dependabot panel with a `tolerable_risk` rationale; this
section restates the reasoning so it survives outside the GitHub UI.

All five rustls/aws/lru entries below clear automatically once
`aws-sdk-s3` can be bumped past `=1.110.0` — the workspace currently
pins it for MSRV reasons. The `glib` entry clears once Tauri bumps
its gtk dependency upstream.

| Advisory | Severity | Crate (transitive root) | Why accepted |
|---|---|---|---|
| [GHSA-82j2-j2ch-gfr8](https://github.com/advisories/GHSA-82j2-j2ch-gfr8) | High | `rustls-webpki 0.101.7` (via `aws-sdk-s3` → `rustls 0.21`) | Vulnerability requires opt-in CRL checking with attacker-controlled CRL bytes. The AWS SDK's S3 TLS path does not enable CRL checking, and we never pass `RevocationOptions` to a verifier; the bug is unreachable from our process. |
| [GHSA-xgp8-3hg3-c2mh](https://github.com/advisories/GHSA-xgp8-3hg3-c2mh) | Low | `rustls-webpki 0.101.7` (same root) | Name-constraint acceptance issue. We do not configure name constraints on the AWS SDK TLS path; the AWS SDK uses a default trust store with no explicit name-constraint extensions. |
| [GHSA-965h-392x-2mh5](https://github.com/advisories/GHSA-965h-392x-2mh5) | Low | `rustls-webpki 0.101.7` (same root) | URI-name-constraint variant of the above. Same reasoning: we do not configure URI name constraints. |
| [GHSA-g59m-gf8j-gjf5](https://github.com/advisories/GHSA-g59m-gf8j-gjf5) | Low | `aws-sdk-s3 1.110.0` (direct, pinned) | Defense-in-depth enhancement around the `region` parameter, not an exploit. Our region values come from server configuration we control; user input never reaches the AWS region parameter. |
| [GHSA-rhfx-m35p-ff5j](https://github.com/advisories/GHSA-rhfx-m35p-ff5j) | Low | `lru 0.12.5` (via `aws-sdk-s3`) | Stacked-Borrows soundness issue in `IterMut` reported under Miri. Not a runtime memory-safety bug. We never call `IterMut` on the lru caches AWS SDK uses internally. |
| [GHSA-wrw7-89jp-8q8g](https://github.com/advisories/GHSA-wrw7-89jp-8q8g) | Medium | `glib 0.18.5` (Linux only, via `tauri` → `tray-icon` → `libappindicator` → `gtk`) | Soundness issue in `VariantStrIter`. We do not call `VariantStrIter` from any of our code. The dep enters the tree only on Linux desktop builds and is wholly internal to the Tauri stack. |

If any of these *become* reachable in our codebase (for example, if
we add CRL checking, take user-controlled region input, or call
`VariantStrIter` directly), revisit the relevant row. Bumping
`aws-sdk-s3` past `=1.110.0` clears the first five in one shot.
