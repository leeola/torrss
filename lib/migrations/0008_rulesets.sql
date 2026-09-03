-- The rulesets a reader writes, and the fields each one reads a name with.
--
-- The id is a slug the application fixes when the ruleset is created and
-- never changes after. `library.identity` and `grab_rulesets.ruleset` both
-- carry it, so a reader who renames a ruleset does not orphan either.
--
-- `inherits` carries no `ON DELETE` clause. Deleting a base that still has
-- children then fails at the database as well as in the registry above it,
-- rather than leaving a child pointing at nothing.
CREATE TABLE rulesets (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    inherits TEXT REFERENCES rulesets(id),
    enabled INTEGER NOT NULL DEFAULT 0
);

-- A field's position is its place in its ruleset, which is the order the
-- parts appear in a well-formed name.
--
-- The position is half the key, so rewriting one ruleset's fields never
-- reaches another's. A rewrite deletes and reinserts rather than updating in
-- place, because a saved ruleset can drop a field as easily as change one.
CREATE TABLE ruleset_fields (
    ruleset TEXT NOT NULL REFERENCES rulesets(id) ON DELETE CASCADE,
    position INTEGER NOT NULL,
    name TEXT NOT NULL,
    part TEXT NOT NULL,
    kind TEXT NOT NULL,
    pattern TEXT,
    required INTEGER NOT NULL,
    identity INTEGER NOT NULL,
    PRIMARY KEY (ruleset, position)
);
