-- The values a reader prefers for one parser field, best first.
--
-- A search says which releases are wanted. These say which of the wanted
-- copies of one release is the best, so a reader states once that 2160p
-- beats 1080p rather than repeating it in every search.
--
-- `field` is a parser field name rather than a search id, so one list holds
-- across every search that reads that field.
--
-- `value` is what the reader typed. It normalizes through the field's kind
-- only when a release is ranked, so a stored list survives a change to the
-- kind the field reads with.
--
-- `position` orders the values, best first. It is contiguous from zero,
-- because a write replaces the whole list for one field.

CREATE TABLE preferences (
    field TEXT NOT NULL,
    position INTEGER NOT NULL,
    value TEXT NOT NULL,
    PRIMARY KEY (field, position)
);
