-- Renames the ruleset tables to search tables.
--
-- A search is one saved query over the values a parser reads, such as one
-- search per show. `ruleset` was the earlier name for the same thing, when
-- the application read as a set of rules a feed passed through.
--
-- Each table also carries a `ruleset` column that names the row it belongs
-- to, so the column follows the table.
--
-- This file rebuilds no table. SQLite rewrites the REFERENCES clause of a
-- child table when the table or the column it names is renamed. That
-- rewrite needs `PRAGMA foreign_keys` on, which sqlx sets on every
-- connection. Migration 0019 rebuilt `rulesets` because a constraint drops
-- only through a rebuild. A rename needs no rebuild.
--
-- The order is forced by `ruleset_test_values`, whose composite key
-- references `ruleset_tests(ruleset, position)`. This file renames the
-- parent's column before the child's own, so each rewrite touches one side.

ALTER TABLE rulesets RENAME TO searches;

ALTER TABLE ruleset_conditions RENAME TO search_conditions;
ALTER TABLE search_conditions RENAME COLUMN ruleset TO search;

ALTER TABLE ruleset_tests RENAME TO search_tests;
ALTER TABLE search_tests RENAME COLUMN ruleset TO search;

ALTER TABLE ruleset_test_values RENAME TO search_test_values;
ALTER TABLE search_test_values RENAME COLUMN ruleset TO search;

ALTER TABLE grab_rulesets RENAME TO grab_searches;
ALTER TABLE grab_searches RENAME COLUMN ruleset TO search;
