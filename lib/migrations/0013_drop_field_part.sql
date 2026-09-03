-- Drops the part a field named, which the application stopped reading.
--
-- A field's tint comes from its position among the ruleset's resolved fields,
-- and matching, identity, and the season pack logic all key on the field name
-- and on its kind. So the column named nothing anything read.
--
-- A new file rather than an edit to 0008_rulesets.sql, because sqlx records
-- the checksum of every applied migration and refuses to start when an
-- applied file changes.
ALTER TABLE ruleset_fields DROP COLUMN part;
