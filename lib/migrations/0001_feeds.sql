-- The feeds this application watches, kept between restarts.
--
-- Keyed by URL rather than by name, because the URL is what a fetch goes to
-- and two names for one URL are one feed. A re-registration of the same URL
-- therefore keeps its id.
--
-- `AUTOINCREMENT` stops a removed id from coming back. An id is handed out in
-- a URL, and a reused one points a check at the wrong feed.
--
-- `auth` holds the credentials as JSON, or NULL for a feed that never got
-- any. Most feeds carry their passkey in the URL and need nothing here.
CREATE TABLE feeds (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    name TEXT NOT NULL,
    url TEXT NOT NULL UNIQUE,
    auth TEXT
);
