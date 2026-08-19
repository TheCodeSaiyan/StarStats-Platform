-- Promoted disambiguation columns for the catalog list row.
--
-- display_name is intentionally non-unique (in-game names duplicate while
-- details differ; canonical_id is the unique reference). 176 of 266 rows
-- share a name, and 10 duplicate groups are identical across every
-- previously-promoted column -- so the list row must show what actually
-- differs. Measured 2026-07-29.
--
-- These are projections of `record -> 'extraction'`, promoted rather than
-- read per-request because the list path must not do an unindexed JSONB
-- extraction (same rationale as the existing promoted columns).
--
-- required_item  backfills a SINGLE value here, not an aggregated list.
-- A first draft used string_agg(DISTINCT ...) across all steps, but real
-- data repeats the same item with inconsistent formatting (with/without
-- quantity, singular/plural -- e.g. "15 Amioshi Plague" vs "Amioshi
-- Plague", "Cave Kopion Horn" vs "Cave Kopion Horns"), so DISTINCT
-- produced visible near-duplicate garbage instead of deduping. Taking the
-- first non-empty item (quantity trimmed off both ends) avoids needing
-- quantity+plural folding logic that would have to be reimplemented
-- identically in Rust to stay in agreement -- two implementations that
-- must agree is a drift risk for no user benefit. Future ingests compute
-- the SAME single value via NewContract::from_bundle, which mirrors the two
-- REGEXP_REPLACE passes below exactly; this backfill covers rows that
-- predate that code. If the two ever diverge, one contract yields different
-- values depending on when it was ingested -- a real-Postgres test asserts
-- they agree by running this very SQL against the Rust result.
--
-- Additive only + byte-immutable post-deploy.
ALTER TABLE contracts ADD COLUMN IF NOT EXISTS first_step_location TEXT;
ALTER TABLE contracts ADD COLUMN IF NOT EXISTS required_item       TEXT;
ALTER TABLE contracts ADD COLUMN IF NOT EXISTS step_count          INT;

-- Backfill from the stored packet. The JSONB path is
-- record -> 'extraction' -> 'steps' (see ContractDetail::from_stored,
-- which deserializes `record` as AdminReviewPacketReq).
UPDATE contracts SET
    step_count = (
        SELECT COUNT(*)::INT
        FROM jsonb_array_elements(record #> '{extraction,steps}') AS s
    ),
    first_step_location = (
        SELECT s ->> 'location'
        FROM jsonb_array_elements(record #> '{extraction,steps}') WITH ORDINALITY AS t(s, ord)
        WHERE s ->> 'location' IS NOT NULL AND s ->> 'location' <> ''
        ORDER BY ord
        LIMIT 1
    ),
    required_item = (
        SELECT btrim(
            regexp_replace(
                regexp_replace(s ->> 'required_item', '^\s*\d+\s*[xX]?\s*', ''),
                '\s*[xX]\s*\d+\s*$', ''
            ),
            E' \t\n\r'
        )
        FROM jsonb_array_elements(record #> '{extraction,steps}') WITH ORDINALITY AS t(s, ord)
        -- Filter on the STRIPPED value, not the raw one. A step whose
        -- required_item is a bare quantity ("25") passes a raw non-empty
        -- test, strips to '', and LIMIT 1 then takes that empty string
        -- instead of falling through to the real item on a later step —
        -- which renders as a blank segment in the catalog row. Verified:
        -- steps ["25", "Prota"] yielded '' before this, 'Prota' after.
        --
        -- The two-argument `btrim(x, ' \t\n\r')` form (rather than the
        -- single-arg `btrim(x)`, which strips ASCII space only) matters
        -- here for the same reason: single-arg btrim leaves a bare tab
        -- ("\t") stripped-but-non-empty, so this filter would pass it
        -- and the catalog would pick a whitespace-only value instead of
        -- falling through to a later, real item -- while Rust's
        -- `str::trim()` (Unicode-whitespace-aware) already reduces it to
        -- "" and skips it. Verified empirically against Postgres 17.
        -- This closes the ASCII-whitespace case (space/tab/newline/CR);
        -- a non-breaking space (U+00A0) is a known, narrower residual
        -- gap -- Postgres's `\s` in `regexp_replace` above doesn't treat
        -- it as whitespace under `en_US.utf8`, so a leading/trailing
        -- NBSP survives the digit-strip regexes themselves, not just
        -- this final trim. Not fixed here: closing it needs the
        -- LEADING/TRAILING_QTY regexes to gain an explicit NBSP
        -- alternation, which is real regex-authoring work, not a
        -- one-line change, and it's a narrower/less-reached case than
        -- the tab one above.
        WHERE btrim(
                  regexp_replace(
                      regexp_replace(coalesce(s ->> 'required_item', ''), '^\s*\d+\s*[xX]?\s*', ''),
                      '\s*[xX]\s*\d+\s*$', ''
                  ),
                  E' \t\n\r'
              ) <> ''
        ORDER BY ord
        LIMIT 1
    )
WHERE jsonb_typeof(record #> '{extraction,steps}') = 'array';
