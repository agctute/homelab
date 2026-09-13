# Configuration Reference

Settings are split between two Kubernetes files:
- **`k8s/configmap.yaml`** — non-sensitive settings (paths, limits, feature flags)
- **`k8s/secret.yaml`** — passwords and credentials

---

## Required credentials (secret.yaml)

These must be filled in before deploying. There are no defaults.

| Variable | What it is |
|----------|------------|
| `MANGADEX_USERNAME` | Your MangaDex account username |
| `MANGADEX_PASSWORD` | Your MangaDex account password |
| `MANGADEX_CLIENT_ID` | Personal client ID from MangaDex settings |
| `MANGADEX_CLIENT_SECRET` | Personal client secret from MangaDex settings |
| `QBITTORRENT_USERNAME` | qBittorrent Web UI username |
| `QBITTORRENT_PASSWORD` | qBittorrent Web UI password |

### Getting your MangaDex personal client

1. Log in at [mangadex.org](https://mangadex.org)
2. Go to **Settings → API Clients**
3. Create a new client — the name doesn't matter
4. Copy the **Client ID** and **Client Secret** into `secret.yaml`

Using a personal client gives you a much higher request limit (~1500/min vs ~40/min unauthenticated), which is necessary for the initial bulk download.

---

## Path settings (configmap.yaml)

| Variable | Default | What it is |
|----------|---------|------------|
| `NAS_COMICS_PATH` | `/books/manga` | Root folder each series gets a subdirectory under — one flat `{series}/` per title, no completed/language split. This is the **same NFS export and layout torrent-allocator uses** (`10.0.0.41:/volume1/books`, `manga` app) so MangaDex downloads and torrent-allocator-routed manga end up in the same library tree — see `apps/torrent-allocator/docs/architecture.md`. |
| `DB_PATH` | `/comics/state.db` | Where the service stores its database (tracks what's been downloaded) |

The shared books library is mounted at `/books` inside the container (the `books` PVC in `k8s/pvc.yaml`, a static NFS PV pointing at the same export torrent-allocator mounts). The sqlite state DB lives on a separate, legacy `/comics` mount (`nas-comics-pvc`) that predates the books-library unification — see the comments in `k8s/pvc.yaml`.

---

## Tuning settings (configmap.yaml)

| Variable | Default | What it is |
|----------|---------|------------|
| `CONCURRENT_DOWNLOADS` | `3` | How many chapters to download at the same time |
| `MAX_API_RPS` | `5` | Maximum MangaDex API requests per second. Keep at 5 or below. |
| `LOG_LEVEL` | `info` | How much detail to log. Options: `error`, `warn`, `info`, `debug` |

---

## Startup triggers (configmap.yaml)

These are one-shot flags useful for the first deployment or debugging. Set them back to `"false"` after the first run.

| Variable | Default | What it does |
|----------|---------|-------------|
| `RUN_CATALOG_REFRESH_ON_START` | `false` | Runs the yearly catalog build immediately when the service starts, instead of waiting until Jan 1 |
| `RUN_DAILY_UPDATE_ON_START` | `false` | Runs the daily chapter check immediately on start |

**First deployment:** Set `RUN_CATALOG_REFRESH_ON_START` to `"true"` so the catalog is populated right away. Once you've confirmed the catalog is built (check logs), set it back to `"false"` and redeploy.

---

## Nyaa / qBittorrent fallback (configmap.yaml)

| Variable | Default | What it is |
|----------|---------|------------|
| `NYAA_ENABLED` | `true` | Set to `"false"` to disable the nyaa fallback entirely |
| `QBITTORRENT_URL` | `http://10.0.0.41:8090` | URL to your qBittorrent Web UI. In this cluster qBittorrent runs via docker-compose directly on the NAS (not as an in-cluster Service) — this must match `QBITTORRENT_URL` in `apps/torrent-allocator/k8s/configmap.yaml`, the source of truth for this value. |
