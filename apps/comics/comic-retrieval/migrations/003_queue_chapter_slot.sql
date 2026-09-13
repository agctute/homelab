-- Tracks which (manga, volume, chapter) slot a queued item targets, so we can
-- detect when MangaDex lists multiple scanlation groups' releases of the same
-- volume/chapter and avoid queuing more than one (they'd otherwise silently
-- overwrite the same file on disk, since the CBZ filename is derived only
-- from title/volume/chapter, not from the group or chapter_id).
ALTER TABLE download_queue ADD COLUMN chapter_num TEXT;
ALTER TABLE download_queue ADD COLUMN volume_num TEXT;
