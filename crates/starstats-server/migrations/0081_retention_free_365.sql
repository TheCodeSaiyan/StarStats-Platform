-- Free-tier event retention: 90 days -> 365 days.
--
-- DATA CHANGE, stated as such: no schema changes here. 0037 seeded
-- free = 90, while the settings page has always told players their
-- events are kept for a year. 365 is the decided window (2026-09-29);
-- supporters stay unlimited (NULL).
--
-- Only moves the seeded value: if an operator has already set the free
-- window to anything other than 90 (through the admin control or SQL),
-- that choice stands. One-way by design: a lengthened window deletes
-- nothing, and there is no reason to restore 90. To go back, set it in
-- the admin console (PUT /v1/admin/retention/policies/free).
UPDATE retention_policies
   SET retention_days = 365, updated_at = NOW()
 WHERE tier = 'free' AND retention_days = 90;
