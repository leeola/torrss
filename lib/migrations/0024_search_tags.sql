-- The client tags a grab of a release this search matches carries.
--
-- The list is stored joined with commas. qBittorrent refuses a comma inside
-- a tag, so no tag holds one and the join loses nothing.
--
-- The empty string is a search that tags nothing, which the default gives
-- every row the table already holds.

ALTER TABLE searches ADD COLUMN tags TEXT NOT NULL DEFAULT '';
