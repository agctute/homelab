-- Tracks per-manga historical-backlog backfills: when MangaDex only hosts a
-- recent tail of a series' chapters (e.g. the last 3 of ~300), the older
-- volumes are fetched in bulk via nyaa instead, then reconciled into the
-- library once the torrent completes. See src/backfill.rs.
CREATE TABLE IF NOT EXISTS backfills (
    manga_id         TEXT PRIMARY KEY,
    status           TEXT NOT NULL DEFAULT 'needed',
    -- JSON array of {volume, first_chapter, last_chapter, chapter_count} for
    -- the volumes MangaDex doesn't host any chapter of.
    volume_map       TEXT NOT NULL,
    torrent_hash     TEXT,
    created_at       TEXT NOT NULL,
    updated_at       TEXT NOT NULL
);
