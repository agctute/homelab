# Code Architecture

A reference for understanding how the pieces fit together, intended for anyone reading or modifying the source.

---

## Project layout

```
comic-retrieval/
├── Cargo.toml            dependencies
├── Dockerfile            multi-stage build
├── migrations/           database schema (runs automatically on startup)
│   ├── 001_initial.sql   core tables
│   └── 002_nyaa.sql      adds nyaa_attempted tracking column
└── src/
    ├── main.rs           startup: auth → worker → scheduler → wait
    ├── config.rs         reads all settings from environment variables
    ├── scheduler.rs      registers the daily and yearly cron jobs
    ├── worker.rs         background loop that processes the download queue
    ├── mangadex/         everything to do with MangaDex
    │   ├── client.rs     HTTP requests, login, token refresh, rate limiting
    │   ├── models.rs     data shapes returned by the MangaDex API
    │   ├── catalog.rs    fetches top-10% lists and daily chapter feeds
    │   └── download.rs   downloads chapter images from MangaDex's servers
    ├── library/          everything to do with saving files
    │   ├── cbz.rs        packages downloaded images into a .cbz archive
    │   └── router.rs     sanitizes titles for use as filenames/folder names
    ├── nyaa/
    │   └── search.rs     searches nyaa.si RSS, picks best torrent match
    ├── qbittorrent/
    │   └── client.rs     talks to the qBittorrent Web API
    └── db/
        └── queries.rs    all database reads and writes
```

---

## Data flow

### Yearly catalog refresh

```
catalog.rs: fetch_top_percent("followedCount")
    → paginate MangaDex /manga?order[followedCount]=desc  (top 10%)
    → return HashMap<manga_id, Manga>

catalog.rs: fetch_top_percent("rating")
    → same for rating order

catalog.rs: refresh_catalog()
    → union both maps
    → exclude Chinese origin (zh, zh-hk)
    → upsert each title into db.catalog
    → for each title with no downloads yet:
        queue_new_manga_chapters() → add all English chapter IDs to download_queue
```

### Daily update

```
catalog.rs: queue_daily_updates(since)
    → for each title in db.catalog:
        GET /manga/{id}/feed?updatedAtSince={since}&translatedLanguage[]=en
        → for each new chapter: insert into download_queue (if not already downloaded)
    → update db.state["daily_last_run"]
```

### Download worker loop

```
worker.rs: process_one()
    → pull next item from download_queue (highest priority first)
    → look up title from db.catalog
    → download.rs: download_chapter()
        → GET /chapter/{id} → metadata
        → GET /at-home/server/{id} → server URL + image list
        → download each page image to a temp folder
    → cbz.rs: create() → zip images + ComicInfo.xml into .cbz
    → {NAS_COMICS_PATH}/{series}/ → same flat per-series layout torrent-allocator
      uses for its `manga` app (see apps/torrent-allocator/docs/architecture.md)
    → move .cbz to NAS
    → mark chapter as downloaded in db
    → if download fails 5 times → nyaa fallback
```

### Nyaa fallback

```
worker.rs: try_nyaa_fallback()
    → check db: has nyaa already been attempted for this manga? if yes, skip
    → nyaa/search.rs: search(title)
        → GET nyaa.si RSS feed for "title" in English-translated category
        → parse results, sort by seeders
    → best_match(): find result whose title contains all keywords from the manga title
    → qbittorrent/client.rs: connect() → login to qBittorrent Web UI
    → add_torrent(torrent_url, save_path)
        → save_path = the manga's folder on the NAS
    → mark manga as nyaa_attempted in db (prevents duplicate torrents)
```

---

## Database tables

| Table | Purpose |
|-------|---------|
| `catalog` | One row per tracked manga: title, origin language, publication status, which list(s) it qualified from, whether nyaa has been tried |
| `downloaded_chapters` | One row per successfully saved chapter: chapter ID, file path on NAS |
| `download_queue` | Pending downloads: chapter ID, attempt count, last error message |
| `state` | Key/value store for things like `daily_last_run` timestamp |

The database lives at `DB_PATH` on the NAS mount so it survives pod restarts.

---

## MangaDex rate limiting

MangaDex asks clients not to exceed their documented limits. The service enforces this with a fixed delay between every API request (controlled by `MAX_API_RPS`). At the default of 5 requests/second, the initial catalog build of ~9,000 titles takes several hours — this is intentional. Rushing it risks getting blocked.

Image downloads (chapter pages) go through MangaDex's volunteer download network, which has separate, more generous limits.

---

## CBZ format

A `.cbz` file is just a ZIP archive containing:
- Page images named `0001.jpg`, `0002.jpg`, etc.
- A `ComicInfo.xml` file with metadata (title, series, volume, chapter number)

The `ComicInfo.xml` is what comic reader apps like Komga and Kavita use to display metadata correctly without needing to parse the filename.

---

## Adding a new source

To add a new download source beyond MangaDex and nyaa:

1. Create a new module under `src/` (e.g. `src/mysite/`)
2. Add a search/download function similar to `nyaa/search.rs`
3. Call it from `worker.rs: try_nyaa_fallback()` as an additional fallback step
4. Add any new config keys to `config.rs` and `k8s/configmap.yaml`
