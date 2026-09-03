-- The columns sit on the feed rather than in a table of their own, because
-- one check is kept per feed and it leaves with the feed.
--
-- A new file rather than an edit to 0001_feeds.sql, because sqlx records the
-- checksum of every applied migration and refuses to start when an applied
-- file changes.
ALTER TABLE feeds ADD COLUMN checked_at TEXT;
ALTER TABLE feeds ADD COLUMN check_items INTEGER;
ALTER TABLE feeds ADD COLUMN check_added INTEGER;
ALTER TABLE feeds ADD COLUMN check_error TEXT;
