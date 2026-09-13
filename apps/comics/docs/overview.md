# Comic Retrieval — Overview

## What it does

This service automatically builds and maintains a manga/manhwa library on your NAS. Once running, you never have to manually hunt for or download anything.

It has three main jobs:

**1. Once a year — build the catalog**
It asks MangaDex for all the manga that people follow the most, and all the manga with the highest ratings. It takes the top 10% from each list, combines them (so a title only needs to appear on one list to qualify), and saves the result as your "catalog" — the collection of things worth having. Chinese manhua is excluded.

**2. Every day at 3 AM — check for new chapters**
For every title in the catalog, it checks if any new chapters have been released since yesterday. New chapters get added to a download queue.

**3. Continuously — download the queue**
A background process works through the download queue, saving each chapter as a `.cbz` file (a standard comic reader format) into the right folder on your NAS. If a chapter can't be found on MangaDex after several tries, it falls back to searching nyaa.si and adds the best torrent match to qBittorrent.

---

## Where files end up

Files are sorted into four folders inside your NAS comics path:

| Folder | Contents |
|--------|----------|
| `Manga/` | Ongoing Japanese manga |
| `Completed Manga/` | Finished Japanese manga |
| `Manwha/` | Ongoing Korean manhwa |
| `Completed Manwha/` | Finished Korean manhwa |

Each title gets its own subfolder. Chapter files are named like:
```
My Hero Academia c0001 (v01).cbz
My Hero Academia c0002 (v01).cbz
```
Chapter number always leads the filename, with volume (when MangaDex provides one) appended after — this keeps chapters sorting correctly even when only some of them have volume data.

---

## How the fallback works

MangaDex hosts the vast majority of chapters directly. But some chapters are "external only" (hosted on publisher sites like Manga Plus) and can't be downloaded from MangaDex. If a chapter fails to download 5 times in a row, the service:

1. Searches nyaa.si for the manga title in the English-translated manga category
2. Picks the result with the most seeders whose title matches the manga name
3. Adds that torrent to qBittorrent, pointed at the correct NAS folder

The nyaa fallback only runs once per title — it won't keep adding duplicate torrents.
