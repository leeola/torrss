-- The column records the scan that wrote the row.
--
-- A new file rather than an edit to 0003_library.sql, because sqlx records
-- the checksum of every applied migration and refuses to start when an
-- applied file changes.
ALTER TABLE library RENAME COLUMN synced_at TO scanned_at;
