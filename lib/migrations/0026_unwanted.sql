-- The values a reader refuses for one parser field.
--
-- A preference list ranks the copies of a wanted release and hides none of
-- them. A refused value hides every release whose field reads it, so a reader
-- states once that a CAM source is never wanted rather than in every search.
--
-- `field` is a parser field name rather than a search id, so one set holds
-- across every search that reads that field.
--
-- `value` is what the reader typed. It normalizes through the field's kind
-- only when a release is judged, so a stored set survives a change to the
-- kind the field reads with.
--
-- The set has no order. The key holds each typed value once per field.

CREATE TABLE unwanted (
    field TEXT NOT NULL,
    value TEXT NOT NULL,
    PRIMARY KEY (field, value)
);
