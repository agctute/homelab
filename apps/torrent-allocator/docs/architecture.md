# Architecture

Generic post-processing service for qBittorrent. It watches for torrents that
have finished downloading, asks an LLM which downstream app the torrent
belongs to (plus its title and language), and hands it off to that app's
handler for conversion, grouping, and placement into the library.

**Everything is copied into the library, never moved.** qBittorrent keeps
seeding directly out of `/torrents` indefinitely, so a completed torrent's
source files must survive untouched — see `fsutil::copy_file`. This does mean
storage usage roughly doubles for anything torrent-allocator touches (one
copy seeding, one copy in the library); that's the intended tradeoff, not an
oversight.

```
torrent-allocator/
├── migrations/           processed_torrents table (idempotency tracking)
└── src/
    ├── main.rs           startup: db → qBittorrent login → spawn watcher
    ├── config.rs         reads all settings from environment variables
    ├── watcher.rs        poll loop: list completed torrents → enrich → classify → gate → dispatch
    ├── fsutil.rs         sanitize/ensure_dir/copy_file helpers shared by every app
    ├── qbittorrent/
    │   └── client.rs     login + list completed torrents via the Web API
    ├── metadata/
    │   ├── mod.rs        BookMetadata type + guess_search_title() release-name cleanup
    │   └── google_books.rs  best-effort Google Books lookup used to enrich classification
    ├── classifier/
    │   ├── mod.rs        Classifier trait + shared types (app id, title, language)
    │   └── openai.rs     ChatGPT-backed implementation
    ├── format/
    │   ├── mod.rs        SourceFormat detection (cbz/cbr/zip/rar/pdf/epub/image-folder/…)
    │   └── convert.rs    shells out to zip/unrar/ebook-convert to convert into a target format
    ├── apps/
    │   ├── mod.rs        AppHandler trait + registry()
    │   ├── visual.rs      shared comics/manga behavior (CBZ, grouped by series dir)
    │   ├── prose.rs       shared novels/textbooks behavior (single file, own dir)
    │   ├── comics.rs      comics = visual.rs, non-Asian origin
    │   ├── manga.rs        manga = visual.rs, Asian origin
    │   ├── novels.rs       novels = prose.rs, target format EPUB
    │   ├── textbooks.rs    textbooks = prose.rs, target format PDF
    │   └── audiobooks.rs   audiobooks = own AppHandler, no conversion, dirs copied intact
    └── db/
        └── queries.rs    processed_torrents reads/writes
```

## Data flow

```
watcher.rs: tick()
    → qbittorrent/client.rs: list_completed()
        → GET /api/v2/torrents/info?filter=completed
    → for each torrent not already in processed_torrents:
        → fsutil::list_extensions(content_path) → actual file extension(s) on disk
        → metadata::guess_search_title(name) → google_books: best-effort search()
            → BookMetadata{title, authors, publisher, description, categories, language}
              or None if the lookup found nothing / failed — never blocks classification
        → classifier: classify(name, category, file_extensions, book_metadata,
                                [registered app ids + descriptions])
            → OpenAI chat completion, forced to strict JSON
              {"app": ..., "title": ..., "language": ..., "reasoning": ...}
        → if language isn't English → mark "skipped-non-english", done
        → look up the AppHandler whose id() matches the returned app
            → found:     handler.handle(torrent, classification, config)
            → not found: mark "unsorted", log for manual triage
        → record the outcome in processed_torrents (keyed by torrent hash)
```

Completion is detected via the qBittorrent Web API (`filter=completed`)
rather than by watching the directory directly — a raw file listing can't
distinguish a finished torrent from one that's still downloading or stalled,
but qBittorrent already tracks that state authoritatively. `content_path`
from the API response must resolve to a real local path inside this pod —
see the NFS mount notes in `k8s/pvc.yaml`.

## Content rules

| App | Destination | Format | Grouping |
|---|---|---|---|
| `comics` | `/volume1/books/comics` | 1 `.cbz` per edition/volume/chapter | all of a series' `.cbz` files share one directory |
| `manga` | `/volume1/books/manga` | 1 `.cbz` per edition/volume/chapter | all of a series' `.cbz` files share one directory |
| `novels` | `/volume1/books/novels` | 1 `.epub` per novel | each novel gets its own directory |
| `textbooks` | `/volume1/books/textbooks` | 1 `.pdf` per textbook | each textbook gets its own directory |
| `audiobooks` | `/volume1/books/audiobooks` | native audio format, unconverted | each audiobook gets its own directory |

English-only: the classifier reports the content's primary language, and
anything that isn't recognized as English (`en`/`eng`/`english`,
case-insensitive) is skipped rather than routed — see `is_english()` in
`watcher.rs`.

**Comics vs. manga is decided by country of origin, not reading direction.**
The classifier infers origin from the author/creator and the work's name
(see the system prompt in `classifier/openai.rs` and the app descriptions in
`apps/comics.rs` / `apps/manga.rs`): anything originating from an Asian
country (Japan, Korea, China, etc. — manga, manhwa, manhua) is `manga`;
everything else (American, European, etc.) is `comics`. This is still
ultimately an LLM judgment call — the metadata enrichment below narrows it,
but doesn't eliminate misclassification when the torrent name is ambiguous
and no metadata match is found.

### File extensions as a classification signal

The torrent's *name* often carries no file extension at all (e.g. a torrent
named after a book title, containing a single `.m4b` inside) — so the
classifier is also given `file_extensions`, the actual extension(s) found in
the downloaded content via `fsutil::list_extensions()`. This is what lets it
reliably distinguish an audiobook from a same-titled text release: an audio
extension (`.m4b`/`.mp3`/...) is a hard fact the LLM is told to defer to over
whatever the title alone would suggest. Relying on the title text for this
turned out not to be reliable enough in practice even with an explicit
prompt — the model would still pick `novels` for an obviously-audio release
until `file_extensions` was added as a separate, structured field.

### Metadata enrichment (`metadata::google_books`)

Scene-release names are a weak signal on their own — `The Fable (2022-2024)
(Digital) (1r0n)` gives the classifier nothing to work with beyond a plain
English-sounding title, even though *The Fable* is a Japanese manga. Before
classifying, `watcher.rs` strips obvious release noise via
`metadata::guess_search_title()` (everything from the first `(`/`[` onward)
and searches Google Books with the remainder. If a match is found, the
resulting `authors`/`publisher`/`categories` are passed to the classifier as
`book_metadata` — a publisher like VIZ Media/Kodansha/Seven Seas/Yen Press or
a Japanese/Korean/Chinese author name is much stronger evidence of origin
than the torrent's own title.

This is best-effort and additive, not load-bearing:
- No API key is required (Google Books' unauthenticated tier); an optional
  `GOOGLE_BOOKS_API_KEY` raises the rate limit if needed.
- `guess_search_title()` is a blunt heuristic (truncate at the first bracket)
  — it won't always land on a clean query, and the search can match the
  wrong book entirely for generic titles. The system prompt tells the LLM to
  sanity-check `book_metadata` against `torrent_name` rather than trust it
  blindly.
- Any failure (no match, network error, rate limit) resolves to `None` and
  classification proceeds exactly as it did before this existed — see
  `GoogleBooksClient::search()`.
- Hardcover's book-search API would be a reasonable second/alternative
  source (better manga/light-novel coverage in some cases) but requires an
  API token; not implemented yet. `metadata::BookMetadata` is provider-
  agnostic, so adding a `metadata::hardcover` module alongside
  `google_books` and picking between them (or merging both) is a contained
  change.

### Format conversion (`format::convert::ExternalConverter`)

When a torrent's files aren't already in the target format, the relevant
handler shells out to real, well-known tools rather than a placeholder:

- **CBZ**: a `.cbz` is just a zip archive, so a folder of loose images is
  `zip`-ed directly; a `.cbr`/`.rar` archive is extracted with `unrar` first,
  then the extracted folder is zipped. A `.zip` that should be a `.cbz` is
  just copied under a renamed `.cbz` extension — no conversion needed since
  they're the same container format.
- **EPUB/PDF**: Calibre's `ebook-convert <input> <output>` handles arbitrary
  ebook formats (mobi, azw3, epub, pdf, txt, ...) in one invocation — the
  output file's extension tells Calibre the target format.

The Dockerfile installs `zip`, `unrar-free`, and `calibre` in the runtime
stage. `unrar-free` is the open-source clone, not RARLAB's proprietary
`unrar` — it doesn't fully support RAR5. If real-world `.cbr`/`.rar` releases
fail to extract, switch to the non-free `unrar` package (requires enabling
the `non-free` apt component).

### Directory/file grouping logic

- `apps/visual.rs` (comics/manga): if the torrent's content is a single
  folder of images, it's treated as one volume/chapter and converted to one
  `.cbz`. If it's a folder containing multiple items (already-packaged
  `.cbz`/`.cbr`/`.zip`/`.rar` files, or per-chapter image subfolders), each
  immediate child is converted/copied independently into the series
  directory. Nesting more than one level deep (e.g. per-volume folders full
  of per-chapter folders) isn't handled yet — such items are logged and left
  in place rather than guessed at.
- `apps/prose.rs` (novels/textbooks): if the torrent's content is a
  directory, the largest file in it is assumed to be the actual
  book/textbook (covers, `.nfo` files, etc. are ignored). This is a rough
  heuristic — revisit if it misfires.
- `apps/audiobooks.rs`: no format conversion — audio formats vary too much
  to pick one target, and transcoding would just degrade quality for no
  reason. A single file is copied as-is; a directory (e.g. one file per
  chapter) is copied intact rather than cherry-picking one file out of it,
  since — unlike novels/textbooks — an audiobook is often legitimately
  multi-file. Non-audio files inside such a directory are left in place with
  a warning rather than guessed at.

## Adding a new app

1. Create `src/apps/<name>.rs` implementing `AppHandler` directly, or — if it
   behaves like an existing category — reuse `visual::VisualLibraryApp` or
   `prose::ProseLibraryApp` the way `comics.rs`/`manga.rs`/`novels.rs`/
   `textbooks.rs` do.
   - `id()` — a short stable string the classifier will return
   - `description()` — plain-English description fed into the LLM prompt so
     it can tell this app apart from the others
   - `handle()` — copy/convert/group the completed torrent into place
2. Register it in `src/apps/mod.rs::registry()`.
3. Add any app-specific config keys to `config.rs` and `k8s/configmap.yaml`.
4. If the destination lives on a NAS share not already mounted, add a
   PV/PVC in `k8s/pvc.yaml` and mount it in `k8s/deployment.yaml`.

Nothing in `watcher.rs` or the classifier needs to change; both work off the
`AppHandler` trait and the registry list.

## Known gaps / open questions

- CBZ/EPUB/PDF filenames are derived from the sanitized torrent/file name,
  not from parsed volume/chapter numbers — fine for now, but multi-chapter
  releases could get less-than-ideal filenames until real
  volume/chapter-number parsing is added.
- Deep nesting (more than one directory level) inside a comics/manga torrent
  isn't walked — see `apps/visual.rs`.
