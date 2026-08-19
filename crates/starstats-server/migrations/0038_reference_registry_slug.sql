-- 0038_reference_registry_slug.sql — URL-safe slug column for the
-- generic reference catalogue (KB v1).
--
-- Adds a nullable `slug` column to `reference_registry`. The cron
-- populates it on the next refresh; the route layer falls back to
-- `class_name` lookup for rows whose slug is still NULL (legacy URLs
-- keep resolving during the cutover).
--
-- Backfill for locations is done inline here because the wiki
-- already supplies a per-location slug field in `metadata.slug`; we
-- reuse that verbatim so existing bookmarks survive across the
-- rollout. Other categories backfill via the next wiki sync cycle.
--
-- Collisions within (category, base_slug) are resolved
-- deterministically in code: `apply_slug_collisions` in
-- `reference_data.rs` sorts the batch by `class_name` first and
-- appends `-2`, `-3`, … to the lexically-later entries. The
-- partial UNIQUE INDEX below enforces this invariant at the DB
-- layer — a collision after the cron's de-dup pass would fail
-- the upsert with a constraint violation and surface as a 5xx
-- with a logged class_name, which is the right "this shouldn't
-- happen" failure mode.
--
-- Per the project migration rule: ADDITIVE ONLY. No DROPs, no NOT
-- NULL on a populated column without a DEFAULT, and the file is
-- byte-immutable post-deploy.

ALTER TABLE reference_registry
    ADD COLUMN IF NOT EXISTS slug TEXT NULL;

-- Backfill locations from the wiki-supplied slug.
--
-- Apply `lower()` BEFORE `regexp_replace` so the character class
-- `[^a-z0-9]+` doesn't accidentally treat uppercase letters as
-- "non-slug" and replace them with hyphens. The Rust
-- `slugify_ascii` function lowercases each ASCII alphanumeric
-- char as part of the same pass; mirroring that here is how we
-- keep the backfill in lockstep with cron-generated slugs (so a
-- wiki location whose `metadata.slug` is "Crusader-Prime" backfills
-- to "crusader-prime", matching what the next sync would write).
UPDATE reference_registry
   SET slug = trim(both '-' from regexp_replace(lower(metadata->>'slug'), '[^a-z0-9]+', '-', 'g'))
 WHERE category = 'location'
   AND slug IS NULL
   AND metadata ? 'slug'
   AND length(coalesce(metadata->>'slug', '')) > 0;

-- Strip any rows whose backfilled slug came out empty (an input
-- of pure punctuation would produce `''` after trimming). Leaving
-- those NULL lets the next sync derive a slug from display_name
-- via the Rust fallback path instead of storing a dud.
UPDATE reference_registry
   SET slug = NULL
 WHERE slug = '';

-- Lookup index + uniqueness for `lower(slug)`. Partial because the
-- column is still nullable for rows pre-dating the cron's first
-- run. The route layer's case-insensitive comparison
-- (`WHERE lower(slug) = lower($2)`) hits this index directly.
CREATE UNIQUE INDEX IF NOT EXISTS reference_registry_cat_slug_lower_idx
    ON reference_registry (category, lower(slug))
 WHERE slug IS NOT NULL;
