-- The template a ruleset is built on.
--
-- A template claims no title and filters no feed. It holds the fields
-- several rulesets share, and each of those replaces only what differs.
--
-- A new file rather than an edit to 0008_rulesets.sql, because sqlx records
-- the checksum of every applied migration and refuses to start when an
-- applied file changes.
ALTER TABLE rulesets RENAME COLUMN inherits TO based_on;
ALTER TABLE rulesets ADD COLUMN template INTEGER NOT NULL DEFAULT 0;
