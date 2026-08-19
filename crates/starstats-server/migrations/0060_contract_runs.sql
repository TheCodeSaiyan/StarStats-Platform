-- Materialised contract runs, derived by
-- `starstats_core::contract_life::derive_contract_runs` from a handle's
-- hud_notification / join_pu / session_end / game_crash stream.
--
-- Why materialise: a derive-on-read-only design would re-fold a handle's
-- entire event history on every read of `/v1/me/stats/contracts`. This
-- table is a recompute-on-dirty cache of that fold, not an archive --
-- retention marks the handle dirty after a purge (see retention.rs), so
-- the next rebuild re-derives from whatever events survived and
-- `contract_runs` never outlives `events`. Same reasoning as the
-- session/records rollups in 0056.
--
-- Schema-change note: `steps` only gets re-serialised when the handle is
-- dirty, so a handle that stops uploading keeps old-shape JSONB
-- indefinitely. Any future change to `ContractStep` or these columns
-- must ship an `UPDATE stat_rollup_state SET contracts_dirty = TRUE`
-- alongside it, or affected handles 500 on read with nothing to heal them.
--
-- Additive-only + byte-immutable post-deploy (see docs/ENGINEERING.md).
CREATE TABLE IF NOT EXISTS contract_runs (
    claimed_handle    TEXT        NOT NULL,
    -- The game's mission id. NOT unique per handle: a re-accept produces
    -- a Superseded run AND a live one sharing this id, and both are real
    -- history. See the surrogate-key note below.
    mission_id        TEXT        NOT NULL,
    accepted_at       TIMESTAMPTZ,
    name              TEXT        NOT NULL,
    state             TEXT        NOT NULL,
    closed_by         TEXT        NOT NULL,
    step_count        INTEGER     NOT NULL DEFAULT 0,
    steps_complete    INTEGER     NOT NULL DEFAULT 0,
    steps_remaining   INTEGER     NOT NULL DEFAULT 0,
    partial_history   BOOLEAN     NOT NULL DEFAULT FALSE,
    connected_server  TEXT,
    closed_at         TIMESTAMPTZ,
    last_event_at     TIMESTAMPTZ,
    -- Full ContractStep list verbatim. Kept as JSONB rather than a child
    -- table: steps are only ever read with their run, never queried
    -- across runs.
    steps             JSONB       NOT NULL DEFAULT '[]'::jsonb,
    rebuilt_at        TIMESTAMPTZ NOT NULL DEFAULT now(),
    -- Surrogate key. The rebuild is DELETE-all-then-INSERT-all per handle,
    -- so there is no upsert-by-natural-key requirement and the PK exists
    -- only for uniqueness. A natural key would have to include
    -- accepted_at (a re-accepted mission yields a Superseded run AND a
    -- live one), but `ContractRun::accepted_at` is `Option<String>` and a
    -- PK column cannot be NULL. The surrogate sidesteps both problems.
    id                BIGSERIAL   PRIMARY KEY,
    CONSTRAINT contract_runs_lc_chk CHECK (claimed_handle = lower(claimed_handle))
);

CREATE INDEX IF NOT EXISTS contract_runs_handle_accepted_idx
    ON contract_runs (claimed_handle, accepted_at DESC);
CREATE INDEX IF NOT EXISTS contract_runs_handle_state_idx
    ON contract_runs (claimed_handle, state);

-- Recompute-on-dirty flag, alongside the existing sessions_dirty.
-- Defaults TRUE so every existing handle materialises on first read.
ALTER TABLE stat_rollup_state
    ADD COLUMN IF NOT EXISTS contracts_dirty BOOLEAN NOT NULL DEFAULT TRUE;
