-- Every torrent the client holds, stored raw.
--
-- The table is a snapshot of the client, and a scan rewrites it whole. A
-- torrent removed there is known only by its absence from the next scan.
--
-- Names are stored as the client reports them, and the engine reads them at
-- query time. A parser edit therefore changes what counts as owned with no
-- rescan. The `library` table this replaces stored the identity a scan had
-- already built, which froze that answer until the next scan.
--
-- `state` is the label of one of the five states the application reports.
-- `error` holds the message of an `Error` state and is NULL otherwise, so a
-- query reads either without parsing a blob.
--
-- `added_at` is nullable because every field the client reports is optional.

CREATE TABLE torrents (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    state TEXT NOT NULL,
    error TEXT,
    size INTEGER NOT NULL,
    progress REAL NOT NULL,
    added_at TEXT,
    scanned_at TEXT NOT NULL
);

DROP TABLE library;
