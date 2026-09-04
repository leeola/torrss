-- Moves every ruleset onto a parser, and drops templates, standalone
-- fields, and field overrides.
--
-- A parser now holds the fields, so a ruleset keeps only the conditions and
-- tests that say which of the names its parser reads it wants. Nothing a
-- reader wrote is lost and no identity in the library changes:
--
--   * A template becomes the parser of the same id.
--   * A ruleset that declared or replaced a field becomes a parser of its
--     own id, carrying the fields it resolved to. A replaced pattern is a
--     regex, and a regex has no condition it converts to.
--   * Every other ruleset points at the parser its template became.
--
-- The order below is forced: `based_on` references `rulesets(id)` with no ON
-- DELETE clause, so a template row cannot go while a ruleset still names it.
--
-- `based_on` stays on the table and holds NULL from here on, because SQLite
-- refuses to drop a column a foreign key constraint names. `parser` is
-- nullable for the same reason, while the store binds it on every insert.

-- (a) Every ruleset that owns fields becomes a parser: a template, a
-- standalone ruleset, and any ruleset that replaced a field.
INSERT INTO parsers (id, name)
SELECT id, name FROM rulesets
WHERE based_on IS NULL OR id IN (SELECT ruleset FROM ruleset_fields);

-- (b) A ruleset with no template carries its own fields already.
INSERT INTO parser_fields
    (parser, position, name, kind, pattern, required, identity, tight)
SELECT ruleset, position, name, kind, pattern, required, identity, tight
FROM ruleset_fields
WHERE ruleset IN (SELECT id FROM rulesets WHERE based_on IS NULL);

-- (c) A ruleset that replaced a field carries the list it resolved to: the
-- template's fields in the template's order, with each same-named own field
-- in place of the inherited one.
INSERT INTO parser_fields
    (parser, position, name, kind, pattern, required, identity, tight)
SELECT
    r.id,
    t.position,
    t.name,
    CASE WHEN o.ruleset IS NULL THEN t.kind ELSE o.kind END,
    CASE WHEN o.ruleset IS NULL THEN t.pattern ELSE o.pattern END,
    CASE WHEN o.ruleset IS NULL THEN t.required ELSE o.required END,
    CASE WHEN o.ruleset IS NULL THEN t.identity ELSE o.identity END,
    CASE WHEN o.ruleset IS NULL THEN t.tight ELSE o.tight END
FROM rulesets r
JOIN ruleset_fields t ON t.ruleset = r.based_on
LEFT JOIN ruleset_fields o ON o.ruleset = r.id AND o.name = t.name
WHERE r.based_on IS NOT NULL
  AND r.id IN (SELECT ruleset FROM ruleset_fields);

-- (d) A template's saved tests describe how its fields read a name, which is
-- the parser's job now. A ruleset's own tests stay with the ruleset.
INSERT INTO parser_tests (parser, position, title)
SELECT ruleset, position, title FROM ruleset_tests
WHERE ruleset IN (SELECT id FROM rulesets WHERE template = 1);

INSERT INTO parser_test_values (parser, position, field, expected)
SELECT ruleset, position, field, expected FROM ruleset_test_values
WHERE ruleset IN (SELECT id FROM rulesets WHERE template = 1);

DELETE FROM ruleset_tests
WHERE ruleset IN (SELECT id FROM rulesets WHERE template = 1);

-- (e) SQLite adds a column with a foreign key only under a NULL default, so
-- the value lands in a second statement.
ALTER TABLE rulesets ADD COLUMN parser TEXT REFERENCES parsers(id);

UPDATE rulesets
SET parser = CASE
    WHEN based_on IS NOT NULL AND id NOT IN (SELECT ruleset FROM ruleset_fields)
    THEN based_on
    ELSE id
END;

-- (f) Nothing names a template now, so the rows and the shape go.
UPDATE rulesets SET based_on = NULL;

DELETE FROM rulesets WHERE template = 1;

ALTER TABLE rulesets DROP COLUMN template;

DROP TABLE ruleset_fields;

-- (g) The identity names the parser, so the column that records which one a
-- library row came from is named for it.
ALTER TABLE library RENAME COLUMN ruleset TO parser;
