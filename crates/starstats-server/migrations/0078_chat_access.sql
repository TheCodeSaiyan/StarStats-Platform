-- Chat access (social phase 6): who may sign in to chat.
--
--   account_restrictions.chat_blocked  a moderator's chat restriction, a
--                                      fifth capability beside the four in
--                                      0066. FALSE for every existing row:
--                                      no one is chat-restricted today.
--   users.age_declared_at              when the player declared they meet
--                                      the minimum age for chat. NULL until
--                                      they do.
--   users.age_declared_minimum         the minimum they declared against,
--                                      so raising it later (a formal game
--                                      rating) asks everyone again.
--
-- A self-declaration is not age assurance under the Online Safety Act; it
-- is evidence for the children's access assessment, recorded with its
-- date. Chat also needs a verified RSI handle.
--
-- One-way: additive only (three columns, NULL or defaulted); an older
-- server never reads them.

ALTER TABLE account_restrictions
    ADD COLUMN IF NOT EXISTS chat_blocked BOOLEAN NOT NULL DEFAULT FALSE;

ALTER TABLE users ADD COLUMN IF NOT EXISTS age_declared_at TIMESTAMPTZ NULL;
ALTER TABLE users ADD COLUMN IF NOT EXISTS age_declared_minimum SMALLINT NULL;
