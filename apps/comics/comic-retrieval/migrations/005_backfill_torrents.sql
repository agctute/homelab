-- One row per torrent added to backfill a specific missing volume. A single
-- manga's gap is now filled by one torrent per volume (nyaa releases manga
-- one torrent per volume, not as combined ranges — see backfill.rs), so a
-- manga with N missing volumes can have up to N rows here.
CREATE TABLE IF NOT EXISTS backfill_torrents (
    id           INTEGER PRIMARY KEY AUTOINCREMENT,
    manga_id     TEXT NOT NULL,
    volume       TEXT NOT NULL,
    torrent_hash TEXT NOT NULL,
    added_at     TEXT NOT NULL,
    UNIQUE(manga_id, volume)
);
