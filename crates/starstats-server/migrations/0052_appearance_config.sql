-- 0052_appearance_config.sql -- Sitewide appearance defaults.
--
-- Singleton table (id = 1, enforced by CHECK), mirroring
-- 0050_waitlist's `waitlist_config` / 0042_ship_matrix_config /
-- 0020_smtp_config. Holds admin-settable defaults for appearance
-- knobs that apply to every signed-out visitor and any signed-in
-- user who hasn't set a personal override (`users.preferences ->>
-- 'theme_wave_speed'`, migration-free JSONB field).
--
-- `theme_wave_speed` controls the duration of the theme-switch wave
-- transition animation: one of `off`, `slow`, `normal`, `fast`
-- (enforced at the route layer, not a DB CHECK -- same pattern as
-- every other closed-vocabulary TEXT column in this codebase, see
-- docs/ENGINEERING.md "Closed-vocabulary enums").
--
-- Additive + byte-immutable post-deploy: IF NOT EXISTS, NOT NULL with
-- a default, seed row via ON CONFLICT DO NOTHING.

CREATE TABLE IF NOT EXISTS appearance_config (
    id               INT         PRIMARY KEY CHECK (id = 1),
    theme_wave_speed TEXT        NOT NULL DEFAULT 'normal',
    updated_at       TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_by       UUID        NULL REFERENCES users(id) ON DELETE SET NULL
);

INSERT INTO appearance_config (id) VALUES (1) ON CONFLICT (id) DO NOTHING;
