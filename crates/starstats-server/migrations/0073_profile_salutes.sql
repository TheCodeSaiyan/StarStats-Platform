-- Social phase 2: o7 salutes, a public per-profile accolade.
--
-- One row per (saluter, target), both case-insensitive, mirroring how
-- friendships and blocks key handles. The count is public; who saluted
-- is not, except to the owner for their own friends.
--
-- One-way: additive only (a new table and its indexes); an older server
-- never reads it.

CREATE TABLE IF NOT EXISTS profile_salutes (
    saluter_handle TEXT        NOT NULL,
    target_handle  TEXT        NOT NULL,
    created_at     TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- One salute per viewer per profile, whatever case either handle arrives in.
CREATE UNIQUE INDEX IF NOT EXISTS profile_salutes_pair_uq
    ON profile_salutes (lower(saluter_handle), lower(target_handle));

-- The public count and the owner's "friends who saluted" both filter on
-- the target.
CREATE INDEX IF NOT EXISTS profile_salutes_target_idx
    ON profile_salutes (lower(target_handle));
