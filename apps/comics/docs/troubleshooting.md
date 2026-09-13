# Troubleshooting

## Checking logs

```bash
kubectl logs -n comics -l app=comic-retrieval -f
```

Add `--tail=100` to see the last 100 lines without following.

---

## Common issues

### "auth failed" on startup

The MangaDex credentials in `secret.yaml` are wrong, or your personal client hasn't been approved yet.

- Double-check `MANGADEX_USERNAME` and `MANGADEX_PASSWORD` by logging into mangadex.org manually
- Go to **Settings → API Clients** and confirm the client status is "approved"
- Make sure you're using the **Client ID** (looks like `personal-client-...`) and not any other ID shown on the page

### Catalog refresh produces 0 manga

Usually means the API request succeeded but the filter excluded everything. Check:
- The `originalLanguage` filter only passes `ja` (Japanese) and `ko` (Korean) — if MangaDex changes their language codes, this would break
- Look for `WARN` lines in the logs; they'll say which manga IDs failed to parse

### Download queue is growing but no files appear on the NAS

The NAS mount may not be working:
```bash
kubectl exec -n comics deploy/comic-retrieval -- ls "/comics/Shared Folder/Shared Comics"
```
If that errors, the PVC isn't mounted correctly. Check `kubectl describe pvc nas-comics-pvc -n comics`.

### "qBittorrent login failed"

- Confirm the Web UI is enabled in qBittorrent settings (Tools → Web UI)
- Check `QBITTORRENT_URL` — it should point to the Web UI port (default 8080), not the torrent port
- Test from inside the cluster: `kubectl exec -n comics deploy/comic-retrieval -- curl http://qbittorrent:8080`

### "no nyaa match found for 'Title'"

nyaa.si didn't return any result whose title contains all the key words from the manga name. This happens with:
- Titles that use uncommon romanisations (e.g. the nyaa release uses "Kimetsu no Yaiba" but the catalog has "Demon Slayer")
- Very new or obscure titles with no nyaa presence

The torrent won't be added and `nyaa_attempted` is set to 1 so it won't try again. You can reset this manually — see "Resetting the nyaa flag" below.

### Chapters downloading as empty files / corrupt CBZ

This is usually a transient MangaDex server issue. The service will retry up to 5 times automatically. If it keeps failing, you'll see the chapter moved to nyaa fallback.

---

## Manually inspecting the database

The database is a standard SQLite file at `DB_PATH` on the NAS. You can open it with any SQLite browser (e.g. [DB Browser for SQLite](https://sqlitebrowser.org/)) to inspect state.

Useful queries:

```sql
-- How many manga are in the catalog?
SELECT COUNT(*) FROM catalog;

-- How many chapters have been downloaded?
SELECT COUNT(*) FROM downloaded_chapters;

-- What's in the download queue right now?
SELECT manga_id, COUNT(*) as pending, MAX(attempts) as max_attempts
FROM download_queue
GROUP BY manga_id
ORDER BY pending DESC;

-- Which manga have had the nyaa fallback triggered?
SELECT manga_id, title FROM catalog WHERE nyaa_attempted = 1;

-- When did the daily update last run?
SELECT value FROM state WHERE key = 'daily_last_run';
```

### Resetting the nyaa flag

If you want the service to try nyaa again for a title (e.g. after fixing the title name in the catalog):

```sql
UPDATE catalog SET nyaa_attempted = 0 WHERE title = 'Your Manga Title';
```

---

## Forcing a re-download

If you want to re-download chapters that were already marked as done (e.g. they're corrupted on disk), delete the entries from `downloaded_chapters` for that manga:

```sql
DELETE FROM downloaded_chapters WHERE manga_id = 'the-manga-uuid';
```

Then the next daily update or catalog refresh will re-queue them.
