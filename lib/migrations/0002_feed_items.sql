-- Every item any feed ever returned, kept raw for the parser to read later.
--
-- A row is keyed by its feed and the tracker's own guid, so a re-fetch
-- updates the row it already wrote. The key carries the feed URL rather
-- than a registry id, because the registry lives in memory and numbers
-- its feeds from the start again after a restart.
CREATE TABLE feed_items (
    id INTEGER PRIMARY KEY,
    feed_url TEXT NOT NULL,
    guid TEXT NOT NULL,
    title TEXT NOT NULL,
    link TEXT NOT NULL,
    published TEXT,
    size INTEGER,
    seeders INTEGER,
    first_seen TEXT NOT NULL,
    last_seen TEXT NOT NULL,
    UNIQUE (feed_url, guid)
);

-- The feed page lists newest first, over every feed at once.
CREATE INDEX feed_items_published ON feed_items (published DESC);
