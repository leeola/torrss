-- no-transaction
-- Rebuilds `rulesets` without the foreign key on `parser`, so a ruleset that
-- reads with a parser the binary carries stores.
--
-- A shipped parser lives in Rust and has no `parsers` row, so the constraint
-- refused every ruleset on one and the editor answered 500 on a fresh
-- database. The constraint duplicated a check the registry already makes:
-- the engine refuses a ruleset whose parser no declaration carries, and
-- `remove_parser` refuses to remove a parser a ruleset reads with. The
-- registry is the table's only writer, so that check stands alone.
--
-- SQLite drops a constraint only by rebuilding the table. The pragma below
-- takes effect because the `-- no-transaction` line above tells sqlx to run
-- this file directly on the connection, and SQLite ignores the pragma inside
-- a transaction. Foreign keys are off so `DROP TABLE rulesets` runs no
-- implicit delete, which otherwise cascades into `ruleset_conditions` and
-- `ruleset_tests`. Create, copy, drop, then rename is the order the SQLite
-- documentation gives, so those child tables keep naming `rulesets`.
--
-- `based_on` goes with the old table. Nothing reads it, and 0018 kept it
-- only under this same SQLite limit.
--
-- `parser` is NOT NULL because 0018 set every row and the store binds it on
-- every insert. The whole script is idempotent, so a crash between COMMIT
-- and the sqlx bookkeeping insert reruns it without harm.

PRAGMA foreign_keys = OFF;

BEGIN;

CREATE TABLE rulesets_rebuilt (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    enabled INTEGER NOT NULL DEFAULT 0,
    parser TEXT NOT NULL
);

INSERT INTO rulesets_rebuilt (id, name, enabled, parser)
SELECT id, name, enabled, parser FROM rulesets;

DROP TABLE rulesets;

ALTER TABLE rulesets_rebuilt RENAME TO rulesets;

COMMIT;

PRAGMA foreign_keys = ON;
