-- 0054_community_user.sql
-- Synthetic system account that owns anonymously-promoted community
-- submissions. Displayed to users as `community` (the display mapping
-- lives in submissions.rs::display_handle, keyed on this row's fixed
-- UUID) -- NOT via this stored handle.
--
-- The stored `claimed_handle` is deliberately `community.system`, which
-- is UNCLAIMABLE: validate_handle (crates/starstats-server/src/
-- validation.rs) only accepts ASCII alphanumeric + `_-`, so the `.`
-- here can never match a real RSI handle a human could register. This
-- is the collision-proofing for the `ON CONFLICT DO NOTHING` seed: if we
-- seeded the raw handle `community`, a real user already holding it would
-- silently skip the insert (unique `users_handle_uq`), the row would
-- never exist, and every anonymous publish would then FK-fail. An
-- unclaimable handle can never be blocked by a real user, so the seed
-- always lands.
--
-- password_hash '!disabled' is not a valid PHC string (Argon2's
-- PasswordHash::new requires the '$'-prefixed PHC format), so
-- users::verify_password's `let Ok(parsed) = PasswordHash::new(phc)
-- else { return false; }` guard rejects it without panicking -- the
-- account can never log in. Verified against the existing
-- `verify_rejects_garbage_phc` unit test in crates/starstats-server/
-- src/users.rs, which covers exactly this non-PHC-string case.
INSERT INTO users (id, email, password_hash, claimed_handle)
VALUES (
    '00000000-0000-0000-0000-0000000b0700',
    'community@system.starstats.invalid',
    '!disabled',
    'community.system'
)
ON CONFLICT DO NOTHING;
