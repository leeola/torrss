-- The client tags a search on this parser carries when it names none of its
-- own.
--
-- The list is stored joined with commas. qBittorrent refuses a comma inside
-- a tag, so no tag holds one and the join loses nothing.
--
-- The empty string is a parser that tags nothing, which the default gives
-- every row the table already holds.

ALTER TABLE parsers ADD COLUMN tags TEXT NOT NULL DEFAULT '';
