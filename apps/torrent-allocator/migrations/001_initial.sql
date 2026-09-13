CREATE TABLE IF NOT EXISTS processed_torrents (
    hash         TEXT PRIMARY KEY,
    name         TEXT NOT NULL,
    app          TEXT NOT NULL,
    status       TEXT NOT NULL,
    detail       TEXT,
    processed_at TEXT NOT NULL
);
