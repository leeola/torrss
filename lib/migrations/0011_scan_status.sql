-- What the last library scan found.
--
-- One row, and the `CHECK` is what keeps it that way. The pages show one
-- result, so a table that could hold two would need a rule for which one
-- wins, and there is no such rule to write.
--
-- The counts and the error are nullable, because a scan ends one way or the
-- other. A stored error means the counts never happened.
CREATE TABLE scan_status (
    id INTEGER PRIMARY KEY CHECK (id = 1),
    scanned_at TEXT NOT NULL,
    torrents INTEGER,
    matched INTEGER,
    error TEXT
);
