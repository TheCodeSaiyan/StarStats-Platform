# Runbook — KEK / secret-key recovery

Covers the at-rest encryption key (KEK) and the JWT signing key: what breaks
when either goes missing or changes, how to tell which failure you're in, and
how to recover. Written after the 2026-07-24 prod outage where a missing KEK
crash-looped the API.

---

## Background — the two on-disk keys

Both live in the **`starstats_api_state`** named volume, mounted at
`/var/lib/starstats` (see `home-servers-build/compose/starstats/compose.yml`).
Losing this volume loses both keys.

| Key | File (default) | Env override | Autogen env | Encrypts / signs |
|-----|----------------|--------------|-------------|------------------|
| **KEK** | `/var/lib/starstats/totp-kek.bin` | `STARSTATS_KEK_FILE` | `STARSTATS_KEK_AUTOGEN` (default `false`) | TOTP secrets **and** the DB-managed SMTP password, both envelope-encrypted at rest |
| **JWT** | `/var/lib/starstats/jwt-key.pem` | `STARSTATS_JWT_KEY_FILE` | `STARSTATS_JWT_KEY_AUTOGEN` (default `false`) | RS256 signing of every user + device token |

Load path (`main.rs`, in this order): KEK first, JWT key second. Both call
`load_or_generate(path, autogen_allowed)`:

- File present, right length → load it.
- File **missing** + autogen **false** → **hard boot failure** (`?` propagates,
  process exits). This is deliberate: a stateless redeploy without the volume
  must fail loudly, not silently re-key.
- File missing + autogen **true** → generate a fresh key, write it, continue.

**A KEK that changes cannot decrypt anything the old one encrypted.** A JWT key
that changes invalidates every existing session.

---

## Symptom → which failure you're in

### A. API crash-loops on boot
```
Error: TOTP KEK missing at /var/lib/starstats/totp-kek.bin and autogen is
disabled. Either mount a persistent volume containing the key, or set
STARSTATS_KEK_AUTOGEN=true ...
```
→ **KEK file is absent and `STARSTATS_KEK_AUTOGEN` is false.** The whole
platform is down (every DB-backed endpoint; `/healthz` never even starts).
Go to **Recovery 1**.

If boot instead fails on the JWT key (`server key missing …`), it's the same
shape one key over — same recovery, `STARSTATS_JWT_KEY_AUTOGEN`.

### B. API boots, but no email is sent
```
WARN  SMTP: DB config read failed; falling back to env-based config
      error="decrypt smtp password"
INFO  SMTP not configured; using noop mailer (no verification emails)
INFO  noop mailer: would send waitlist invite  to=… token=…
```
→ **KEK file is present but is a _different_ key than the one that encrypted
the stored SMTP password.** Boot succeeds (KEK loads fine — it's a valid key,
just the wrong one), but the DB SMTP config can't be decrypted, so the mailer
falls through to no-op and every send is silently swallowed. Go to
**Recovery 2**.

### C. `/admin/smtp` shows "Something went wrong"
→ Same root as B. `GET /v1/admin/smtp` decrypts the stored password just to
render the (redacted) form, so a KEK mismatch 500s the page — the recovery UI
is blocked by the very key it would fix. **Recovery 2** clears it.

### D. Users unexpectedly logged out after a deploy
→ The JWT key changed (volume loss + `STARSTATS_JWT_KEY_AUTOGEN=true`
regenerated it silently). Harmless on a single-user instance; a signal of
volume loss on a multi-user one. See **Root cause** below.

---

## Recovery 1 — API won't boot (missing KEK)

> **This instance keeps KEK autogen OFF on purpose.** `home-servers-build`
> commit `3f0bc6f` (2026-07-23) deliberately removed `STARSTATS_KEK_AUTOGEN`
> so a missing key fails loudly instead of silently re-keying. The crash-loop
> is that guard working as designed. Do NOT "fix" it by turning autogen back
> on permanently — that reinstates the exact silent re-keying that commit
> removed and quietly destroys TOTP + SMTP secrets. Recover the real key
> first; only re-key as a deliberate, one-time act (last section).

### Step 1 — find the real key (it is probably not gone)

A *named* volume doesn't lose data on container recreation. If `totp-kek.bin`
vanished, the most likely cause is a **compose project-name mismatch**: Docker
namespaces volumes as `<project>_starstats_api_state`, so a manual
`docker compose -f …` run from a different directory or with a different `-p`
than Komodo uses will create/point at a *different, empty* volume — and the API
boots against the empty one while your original key sits untouched in the real
volume.

```bash
# List every candidate volume (there may be more than one):
docker volume ls | grep starstats_api_state

# Inspect each for the key files:
for v in $(docker volume ls -q | grep starstats_api_state); do
  echo "== $v =="; docker run --rm -v "$v":/x alpine ls -la /x 2>/dev/null
done
```

If one volume holds `totp-kek.bin` + `jwt-key.pem`, that's your original key.
Recover by making the stack use that volume:
- Match Komodo's compose **project name** (the `<project>_` prefix on the good
  volume), or
- Copy the key across:
  `docker run --rm -v GOOD_VOL:/src -v ACTIVE_VOL:/dst alpine sh -c 'cp -a /src/. /dst/'`

Redeploy; boot succeeds with TOTP + SMTP intact and the fail-loud posture
preserved. **No re-key, no config change.**

### Step 2 — only if the key is truly unrecoverable

No stray volume has it and there is no off-host backup → the ciphertext it
protected (TOTP secrets, DB SMTP password) is permanently unreadable and a
re-key is unavoidable. Do it deliberately, and keep autogen OFF afterwards:

**Option A — one-time autogen, then re-lock (mirrors the JWT twin's history):**
1. Temporarily add `STARSTATS_KEK_AUTOGEN: "true"` next to `STARSTATS_KEK_FILE`.
2. `docker compose -f compose/starstats/compose.yml up -d --force-recreate starstats-api`
   — boot logs `generated new TOTP KEK`; the key writes to the volume.
3. **Remove the flag again** and redeploy, restoring the fail-loud posture
   (re-commit the `3f0bc6f` state). The key persists and is reused.

**Option B — generate out-of-band, no flag flip:** write a 32-byte random key
into the volume directly, leaving autogen off throughout:
```bash
docker run --rm -v ACTIVE_VOL:/x alpine sh -c \
  'head -c 32 /dev/urandom > /x/totp-kek.bin && chmod 600 /x/totp-kek.bin'
```
then redeploy. (Only valid because the old key is gone — never overwrite a live
KEK this way.)

Either way, do the post-regeneration cleanup below, and **back the new key up
off-host immediately** so the next volume loss is a restore, not a re-key.

## Recovery 2 — booted but SMTP won't send (KEK mismatch)

The stored SMTP password ciphertext can't be decrypted under the current KEK
and is dead weight. Clear it so reads stop failing, then re-enter it (the PUT
re-encrypts under the current KEK and hot-swaps the live mailer — no restart):

```sql
UPDATE smtp_config SET password_ciphertext = NULL, password_nonce = NULL WHERE id = 1;
```
Run as `starstats_app`. This satisfies the both-NULL CHECK, so `/admin/smtp`
renders again. Re-enter the password there, ensure **enabled** is ticked, Save,
then use the page's **Send test email** button to confirm delivery end-to-end
(check spam — first sends from a new setup often land there).

**More durable option:** set `SMTP_URL` (+ `SMTP_WEB_ORIGIN`) in the compose so
mail delivery bypasses the KEK entirely and survives any future re-key. The
DB-managed `/admin/smtp` config then becomes optional rather than the only path.

## Post-regeneration cleanup (after any KEK change)

1. **SMTP password** — undecryptable under the new key. Do Recovery 2.
2. **TOTP enrolments** — undecryptable; affected users must re-enroll 2FA.
3. **Sessions** (only if the JWT key also regenerated) — everyone re-logs-in.

## Stranded waitlist invites

A KEK/SMTP outage during auto-admit mints an invite token and sets
`admitted_at`, but the invite email no-ops — leaving people "admitted" with no
link. Recovery:

- **Immediate, single row:** build the link by hand from the DB row —
  `https://starstats.app/auth/signup?invite=<invite_token>`. The token doesn't
  expire and isn't consumed until used.
- **Via the console:** `/admin/waitlist` → the **Admitted** table → select the
  row → **Resend** (re-sends the *existing* token, never re-mints, and reports
  the count of *successful* sends). Note: re-**admit** does NOT resend an
  already-admitted row (`admit_batch` is `WHERE admitted_at IS NULL`) — use
  Resend, not Admit.

---

## Root cause — the loud failure is telling you the volume "moved" or was wiped

`starstats_api_state` is a **named** Docker volume; it should survive container
recreation. If `totp-kek.bin` vanished, the key almost certainly wasn't
destroyed — the API booted against a *different* volume. In likelihood order:

1. **Compose project-name mismatch (prime suspect).** Volumes are namespaced
   `<project>_starstats_api_state`, where `<project>` defaults to the compose
   directory name (or `-p`). A manual `docker compose -f …` run from a
   different path than Komodo uses points at a different, empty volume. The
   original key is intact under the other prefix — see Recovery 1 Step 1. This
   is the most probable cause any time the loss coincides with hands-on
   `docker compose` work (as the 2026-07-24 outage did — it followed a session
   of manual SMTP debugging).
2. **Volume actually removed.** `docker compose down -v`, `docker volume prune`,
   or a Komodo redeploy set to recreate volumes.

`STARSTATS_JWT_KEY_AUTOGEN=true` means the JWT key has been *silently*
regenerating on every such event (logging everyone out) — the KEK failing loud
is the ONLY reason this class of failure was ever visible. That is why the fix
is not "turn KEK autogen on to match": that would blind you to it exactly like
the JWT key already is.

So: **once unblocked, pin down which volume the stack should own and always
deploy it the same way.** Standardise the compose project name (Komodo's), and
never hand-run `docker compose` for this stack from an ad-hoc directory. Then
back up `totp-kek.bin` + `jwt-key.pem` off-host (CA-grade material — the volume
comment already says so), so a future loss is a restore, not a re-key.

## Prevention checklist

- [ ] Deploy the stack with a **consistent compose project name** (verify the
      active volume is `<komodo-project>_starstats_api_state`); never hand-run
      `docker compose` for it from an ad-hoc directory.
- [ ] Verify persistence for real: `down` then `up` (WITHOUT `-v`) keeps
      `totp-kek.bin`.
- [ ] KEK autogen stays **off** (fail-loud, per `3f0bc6f`); JWT autogen is on
      only because a fresh volume was accepted once — consider turning it off
      too now that a key exists, so JWT loss is also loud.
- [ ] Off-host backup of both key files (there is currently **none** — this is
      why the 2026-07-24 loss forced a re-key).
- [ ] `SMTP_URL` set so email doesn't depend on the KEK.
- [ ] Deploy tooling does NOT `down -v` or prune volumes.
