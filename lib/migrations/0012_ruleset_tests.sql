-- The titles a reader saved as tests, and what each one expects a field to
-- read.
--
-- A test belongs to its ruleset and leaves with it, so both tables cascade.
-- The values cascade from the test rather than from the ruleset, so deleting
-- one test takes its own expectations and nothing else.
--
-- The expectations sit in a table of their own rather than in one blob on the
-- test. A field name is a key the reader edits, and a blob hides it from
-- every query that would otherwise find it.
--
-- `position` orders the tests as the editor showed them. It is contiguous
-- from zero, because a save rewrites the whole list.
CREATE TABLE ruleset_tests (
    ruleset TEXT NOT NULL REFERENCES rulesets(id) ON DELETE CASCADE,
    position INTEGER NOT NULL,
    title TEXT NOT NULL,
    PRIMARY KEY (ruleset, position)
);

CREATE TABLE ruleset_test_values (
    ruleset TEXT NOT NULL,
    position INTEGER NOT NULL,
    field TEXT NOT NULL,
    expected TEXT NOT NULL,
    PRIMARY KEY (ruleset, position, field),
    FOREIGN KEY (ruleset, position) REFERENCES ruleset_tests(ruleset, position) ON DELETE CASCADE
);
