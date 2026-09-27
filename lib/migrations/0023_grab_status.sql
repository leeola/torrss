-- The HTTP status the tracker answered a failed download with.
--
-- A 404 or a 410 says the tracker no longer serves the release, where a 403
-- or a 5xx says nothing about the release itself. The error text reads the
-- same to a person, so the status is kept apart for the page to act on.
--
-- Null when the grab succeeded, or when it failed at another stage, such as
-- an unreachable tracker or a client that refused the torrent.

ALTER TABLE grabs ADD COLUMN status INTEGER;
