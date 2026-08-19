-- 0019_staff_roles.sql -- Site-wide staff role grants.
--
-- Distinct from SpiceDB org-level roles (owner/admin/member of an
-- organisation). These are platform-level grants: a `moderator` may
-- accept/reject submission queue items, an `admin` may also grant /
-- revoke roles and reach every other admin endpoint.
--
-- The first admin is bootstrapped from STARSTATS_BOOTSTRAP_ADMIN_HANDLES
-- (comma-separated handles, idempotent) on every server startup. Once
-- there is at least one admin, further grants land here through the
-- admin UI and are mirrored into audit_log.
--
-- Revocation is a soft delete: rows are never removed, just stamped
-- with `revoked_at` + `revoked_by_user_id`. This keeps the partial
-- unique index honest (one *active* grant per user/role) without
-- losing the trail of "who used to be a moderator". Querying the
-- active set is a `WHERE revoked_at IS NULL` against the partial
-- index, which Postgres can answer from the index alone.

CREATE TABLE IF NOT EXISTS staff_roles (
    id                  UUID        PRIMARY KEY,
    user_id             UUID        NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    role                TEXT        NOT NULL,
    granted_at          TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    -- NULL when granted by the bootstrap path (STARSTATS_BOOTSTRAP_ADMIN_HANDLES).
    -- Set when an existing admin grants the role through the UI.
    granted_by_user_id  UUID        REFERENCES users(id) ON DELETE SET NULL,
    revoked_at          TIMESTAMPTZ,
    revoked_by_user_id  UUID        REFERENCES users(id) ON DELETE SET NULL,
    -- Free-form note from the granter / revoker. Optional. Surfaced
    -- in the audit-log JSON payload too, so this field is just for
    -- the active-roles list view.
    reason              TEXT,
    CHECK (role IN ('moderator', 'admin')),
    -- A revocation always names a revoker; an active grant never has one.
    CHECK ((revoked_at IS NULL) = (revoked_by_user_id IS NULL))
);

-- One active grant per (user, role). A revoked row drops out of this
-- index, so re-granting the same role after revoke is a fresh insert.
CREATE UNIQUE INDEX IF NOT EXISTS staff_roles_active_uq
    ON staff_roles (user_id, role)
    WHERE revoked_at IS NULL;

-- Look up "is this user a staff member" without a sequential scan.
CREATE INDEX IF NOT EXISTS staff_roles_user_active_idx
    ON staff_roles (user_id)
    WHERE revoked_at IS NULL;

-- "Show me everyone with a given role" -- used by the admin user list.
CREATE INDEX IF NOT EXISTS staff_roles_role_active_idx
    ON staff_roles (role, granted_at DESC)
    WHERE revoked_at IS NULL;
