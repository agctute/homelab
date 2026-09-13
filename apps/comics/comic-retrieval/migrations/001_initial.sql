CREATE TABLE IF NOT EXISTS catalog (
    manga_id   TEXT PRIMARY KEY,
    title      TEXT NOT NULL,
    origin     TEXT NOT NULL,
    status     TEXT NOT NULL,
    in_follows INTEGER NOT NULL DEFAULT 0,
    in_rating  INTEGER NOT NULL DEFAULT 0,
    added_at   TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS downloaded_chapters (
    chapter_id    TEXT PRIMARY KEY,
    manga_id      TEXT NOT NULL,
    chapter_num   TEXT,
    volume_num    TEXT,
    cbz_path      TEXT NOT NULL,
    downloaded_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS download_queue (
    id         INTEGER PRIMARY KEY AUTOINCREMENT,
    chapter_id TEXT NOT NULL UNIQUE,
    manga_id   TEXT NOT NULL,
    priority   INTEGER NOT NULL DEFAULT 0,
    attempts   INTEGER NOT NULL DEFAULT 0,
    last_error TEXT,
    queued_at  TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS state (
    key   TEXT PRIMARY KEY,
    value TEXT NOT NULL
);
