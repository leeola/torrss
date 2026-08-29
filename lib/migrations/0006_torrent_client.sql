-- The qBittorrent connection this application opens.
--
-- One row, and the `CHECK` is what keeps it that way. The services carry one
-- client, so a table that could hold two would need a rule for which one
-- wins, and there is no such rule to write.
--
-- Every column is nullable. A declaration sets what it names and leaves the
-- rest, which is what lets one file declare the address and another declare
-- only the password.
CREATE TABLE torrent_client (
    id INTEGER PRIMARY KEY CHECK (id = 1),
    url TEXT,
    username TEXT,
    password TEXT
);
