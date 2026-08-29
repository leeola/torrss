-- Which rulesets claimed the release each grab took.
--
-- A grab records that a release was taken. This records why: the rulesets
-- the title passed, so a reader sees which rules acted rather than only that
-- something did.
--
-- `position` keeps the order the engine ranked them in, most specific first.
-- A join returns rows in no order of its own, and the order is the part that
-- says which ruleset won.
CREATE TABLE grab_rulesets (
    item_id INTEGER NOT NULL REFERENCES grabs(item_id) ON DELETE CASCADE,
    ruleset TEXT NOT NULL,
    position INTEGER NOT NULL,
    PRIMARY KEY (item_id, ruleset)
);
