-- The identities the torrent client already holds.
--
-- A sync rewrites this table whole, because it records a snapshot of the
-- client rather than a stream of changes. A torrent removed there is known
-- only by its absence from the next snapshot.
CREATE TABLE library (
    identity TEXT PRIMARY KEY,
    ruleset TEXT NOT NULL,
    torrent_id TEXT NOT NULL,
    name TEXT NOT NULL,
    synced_at TEXT NOT NULL
);
