-- The latest attempt to grab each stored item.
--
-- A grab is an attempt rather than an event, so one row covers an item and
-- a retry overwrites it. A history would grow without bound and the page
-- shows only the last result.
--
-- A null error marks an attempt the torrent client accepted. The text of a
-- failure is kept, because that is what a page has to show to explain it.
CREATE TABLE grabs (
    item_id INTEGER PRIMARY KEY REFERENCES feed_items(id),
    grabbed_at TEXT NOT NULL,
    error TEXT
);
