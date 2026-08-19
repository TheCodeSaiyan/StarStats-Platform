-- Admin-managed runtime config for the RSI Ship Matrix enrichment.
--
-- Singleton row (id = 1, enforced by CHECK), mirroring 0020_smtp_config.
-- Holds the media kill-switch that was previously the
-- STARSTATS_SHIP_MATRIX_MEDIA env var, so an admin can flip official-image
-- rendering on/off from the admin UI with no redeploy (comply-on-request).
--
-- Additive + byte-immutable post-deploy: IF NOT EXISTS, NOT NULL with a
-- default, seed row via ON CONFLICT DO NOTHING.

CREATE TABLE IF NOT EXISTS ship_matrix_config (
    id            INT PRIMARY KEY CHECK (id = 1),
    -- The media kill-switch. FALSE = ship-dark (specs/description still
    -- populate; the media proxy 404s every image). Default FALSE so a
    -- fresh deploy never surfaces RSI images until an admin opts in.
    media_enabled BOOLEAN NOT NULL DEFAULT FALSE,
    updated_at    TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_by    UUID NULL REFERENCES users(id) ON DELETE SET NULL
);

INSERT INTO ship_matrix_config (id) VALUES (1) ON CONFLICT (id) DO NOTHING;
