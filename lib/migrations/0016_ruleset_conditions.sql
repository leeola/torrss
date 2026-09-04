-- The comparisons a ruleset makes on the values its regex read.
--
-- The regex decides which titles have the shape the ruleset describes, and
-- these decide which of those it wants. A ruleset with no row here claims
-- every title its regex reads.
--
-- A condition belongs to its ruleset and leaves with it, so the table
-- cascades.
--
-- `field` names one of the ruleset's resolved fields, and `op` is the same
-- text the editor's form posts, so one vocabulary serves the form and the
-- table. `value` is what the reader typed, which normalizes through the
-- field's kind before an equality compares it.
--
-- `position` orders them as the editor shows them. It is contiguous from
-- zero, because a save rewrites the whole list.
CREATE TABLE ruleset_conditions (
    ruleset TEXT NOT NULL REFERENCES rulesets(id) ON DELETE CASCADE,
    position INTEGER NOT NULL,
    field TEXT NOT NULL,
    op TEXT NOT NULL,
    value TEXT NOT NULL,
    PRIMARY KEY (ruleset, position)
);
