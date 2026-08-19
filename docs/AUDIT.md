# Audit Log

Audit ≠ logs. Logs answer "what happened"; audit answers "who did
what to which resource at when, and can we prove it later." Audit
must be **append-only**, **tamper-evident**, and **retained beyond
operational logs**.

## Storage

Three layers of redundancy:

1. **Postgres `audit_log` table** — primary, queryable
2. **Loki** — operational query plane (joinable with regular logs)
3. **MinIO bucket `starstats-audit` with Object Lock in Compliance mode** — long-term immutability

The Postgres table is the **system of record**. Loki and MinIO are
mirrors. If Postgres ever conflicts with Loki, Postgres wins.

## Schema

```sql
CREATE TABLE audit_log (
    id            BIGSERIAL PRIMARY KEY,
    occurred_at   TIMESTAMPTZ NOT NULL DEFAULT now(),
    actor_type    TEXT NOT NULL,    -- user | service | system | oauth_app
    actor_id      TEXT NOT NULL,
    action        TEXT NOT NULL,    -- e.g. "org.member.added"
    resource_type TEXT NOT NULL,
    resource_id   TEXT NOT NULL,
    metadata      JSONB NOT NULL DEFAULT '{}',
    ip_address    INET,
    user_agent    TEXT,
    request_id    TEXT NOT NULL,    -- trace_id when present
    prev_hash     BYTEA NOT NULL,
    row_hash      BYTEA NOT NULL,
    CHECK (length(prev_hash) = 32),
    CHECK (length(row_hash)  = 32)
);

-- Block UPDATE and DELETE permanently
REVOKE UPDATE, DELETE ON audit_log FROM PUBLIC;
REVOKE UPDATE, DELETE ON audit_log FROM starstats_app;

-- Trigger-enforced refusal (defence in depth)
CREATE OR REPLACE FUNCTION audit_log_immutable() RETURNS trigger AS $$
BEGIN
  RAISE EXCEPTION 'audit_log is append-only';
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER audit_log_no_update
  BEFORE UPDATE ON audit_log FOR EACH ROW EXECUTE FUNCTION audit_log_immutable();
CREATE TRIGGER audit_log_no_delete
  BEFORE DELETE ON audit_log FOR EACH ROW EXECUTE FUNCTION audit_log_immutable();
```

## Hash chain

Each row's `row_hash` includes the previous row's `row_hash`, so any
historical tampering breaks every subsequent hash. Verification is a
single linear scan.

```
row_hash[N] = sha256(prev_hash[N] || canonical_json(occurred_at, actor_type,
                       actor_id, action, resource_type, resource_id,
                       metadata, ip_address, user_agent, request_id))

prev_hash[N] = row_hash[N-1]   (or 32 zero bytes for the first row)
```

`canonical_json` = JSON with sorted keys, no whitespace, UTF-8.

Verification job runs nightly; mismatches alert via GlitchTip with
`level=fatal`.

## What gets audited

| Category | Actions |
|---|---|
| Authentication | `auth.login.success`, `auth.login.failure`, `auth.mfa.enrolled`, `auth.mfa.used`, `auth.session.revoked` |
| Identity | `user.created`, `user.email.changed`, `user.deleted`, `user.rsi.linked`, `user.rsi.unlinked` |
| Authorization | `perm.role.assigned`, `perm.role.revoked`, `perm.scope.granted`, `perm.scope.revoked` |
| Org | `org.created`, `org.deleted`, `org.member.added`, `org.member.removed`, `org.member.role_changed` |
| Sharing | `share.created`, `share.revoked`, `visibility.changed`, `public.toggle` |
| Data | `data.exported`, `data.deleted`, `events.ingest_throttled` (when limits hit) |
| OAuth apps | `oauth_app.registered`, `oauth_app.deleted`, `token.issued`, `token.revoked`, `token.scope_changed` |
| Admin | `admin.config.changed`, `admin.user.impersonated` |

NOT in audit (these are operational logs only):

- Successful read requests
- Telemetry emission
- Background job ticks
- Health checks

The bar is "could this matter in a security review or dispute?"

## Mirroring

The API server writes audit rows in the **same Postgres transaction**
as the action being audited. If the action commits, the audit row
commits. If either fails, both roll back.

```rust
// Pseudo, real impl in starstats-server/src/audit/mod.rs
let mut tx = pool.begin().await?;
do_action(&mut tx, ...).await?;
audit::record(&mut tx, AuditEvent { ... }).await?;
tx.commit().await?;
```

Mirroring to Loki and MinIO happens **asynchronously** via a
background task that scans `audit_log.id > last_mirrored_id`. If the
mirror fails, the source-of-truth row remains in Postgres and is
retried — we never block the user-facing action on mirror health.

## Retention

| Store | Retention |
|---|---|
| Postgres `audit_log` | 90 days (operational query window) |
| Loki | 90 days |
| MinIO `starstats-audit` (Object Lock) | 7 years (set at bucket creation, immutable) |

**Policy:** Postgres retains `audit_log` rows for 90 days; older rows
are pruned. The MinIO mirror is the long-term system of record and
retains 7 years via Object Lock in compliance mode (set at bucket
creation by the operator's MinIO setup script). Compliance-mode
locks cannot be
shortened or removed even by the bucket owner, so MinIO is the trust
anchor for any after-the-fact audit query that exceeds 90 days.

### Pruning job

The pruning DELETE is permitted because the trigger-enforced
append-only rule blocks UPDATE/DELETE for non-superusers; the
scheduled job runs as the Postgres superuser inside the container.

```sql
DELETE FROM audit_log
 WHERE occurred_at < now() - interval '90 days';
```

Schedule via host cron on the docker host (weekly is sufficient — the
audit_log table is small and the prune is cheap):

```cron
# /etc/cron.weekly/starstats-audit-prune
0 4 * * 0 docker exec postgres psql -U postgres -d starstats \
    -c "DELETE FROM audit_log WHERE occurred_at < now() - interval '90 days';"
```

`pg_cron` is **not** used here because the existing voyager Postgres
image doesn't have the extension installed, and adding it for a
single weekly DELETE isn't worth the operational surface. Host cron
+ `docker exec` keeps the dependency footprint flat.

### Account deletion and event continuity

Deleting a user (`DELETE /v1/auth/me`) removes the row in `users` and
cascades to `devices` and `device_pairings`, but **does not** delete
the user's rows in `events`. The `events` table is keyed by
`claimed_handle`, not `user_id`, and there is no FK from `events` to
`users`. This is intentional: the events history is part of the
audit-relevant record, and preserving it means re-pairing the same
handle later restores the user's stats continuously across the gap.
If a specific user ever requires right-to-erasure of their events,
that's handled as a separate, explicit redaction job — never as a
side effect of account deletion. See
`crates/starstats-server/src/users.rs::PostgresUserStore::delete_user`
for the in-source rationale.

### Long-term archive

Postgres rows older than 90 days are recoverable from MinIO if needed
(the mirror is written in the same transaction window as the source
row). Restoring archived rows back into Postgres is **not** an
automated path — pull from MinIO directly when you need them.

## Access control

- **Application code** (`starstats_app` role): INSERT only.
- **Read-only audit role** (`starstats_audit_reader`): SELECT only,
  used by ops dashboards.
- **Application admin** never has UPDATE/DELETE permission anywhere.

Reading the audit log is itself audited (`audit.read` event).
