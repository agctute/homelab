# Deployment Guide

## Prerequisites

- A Kubernetes cluster with access to your NAS (via NFS or similar)
- A storage class that supports NFS mounts (e.g. `nfs-client`)
- qBittorrent running somewhere accessible from the cluster (for the nyaa fallback)
- Docker installed locally to build the image

---

## Step 1 — Fill in credentials

Edit `k8s/secret.yaml` with your real values:

```yaml
stringData:
  MANGADEX_USERNAME: "your-username"
  MANGADEX_PASSWORD: "your-password"
  MANGADEX_CLIENT_ID: "personal-client-..."
  MANGADEX_CLIENT_SECRET: "..."
  QBITTORRENT_USERNAME: "admin"
  QBITTORRENT_PASSWORD: "your-qbt-password"
```

See [configuration.md](configuration.md) for how to get the MangaDex client ID and secret.

---

## Step 2 — Set your NAS paths

Edit `k8s/pvc.yaml` and set the `storageClassName` to match what your cluster uses for NFS:

```yaml
spec:
  storageClassName: nfs-client   # change this to your storage class
  resources:
    requests:
      storage: 5Ti               # adjust to your available space
```

Edit `k8s/configmap.yaml` if your NAS folder layout differs from the default. By
default this points at the same `/volume1/books` NFS export that
`apps/torrent-allocator` uses, under its `manga` app subdirectory:

```yaml
NAS_COMICS_PATH: "/books/manga"
```

---

## Step 3 — Build the Docker image

From the repo root:

```bash
docker build -t comic-retrieval:latest apps/comics/comic-retrieval/
```

Load it into your cluster however you normally do (e.g. `k3s ctr images import`, a local registry, or Docker Hub).

---

## Step 4 — First-time deploy

Enable the startup catalog refresh so the service populates your library immediately instead of waiting until Jan 1:

```yaml
# k8s/configmap.yaml
RUN_CATALOG_REFRESH_ON_START: "true"
```

Apply everything:

```bash
kubectl apply -f apps/comics/k8s/
```

Watch the logs to confirm it's running:

```bash
kubectl logs -n comics -l app=comic-retrieval -f
```

You should see:
1. `MangaDex auth OK` — credentials worked
2. `starting yearly catalog refresh` — building the title list
3. `catalog refresh complete: N manga upserted` — catalog is ready
4. `download worker started` — chapters are downloading

---

## Step 5 — After the first run

Once the catalog is built (you'll see the "catalog refresh complete" log line), flip the startup flag off so it doesn't re-run the full catalog build every time the pod restarts:

```yaml
# k8s/configmap.yaml
RUN_CATALOG_REFRESH_ON_START: "false"
```

Re-apply:

```bash
kubectl apply -f apps/comics/k8s/configmap.yaml
kubectl rollout restart deployment/comic-retrieval -n comics
```

From this point the service runs on its own schedule.

---

## Ongoing operations

**Check what's downloading:**
```bash
kubectl logs -n comics -l app=comic-retrieval --tail=50
```

**Restart the service:**
```bash
kubectl rollout restart deployment/comic-retrieval -n comics
```

**Force a catalog refresh outside of Jan 1:**
Set `RUN_CATALOG_REFRESH_ON_START: "true"`, apply the configmap, then restart the deployment. Set it back to `"false"` afterwards.

**Force a daily update check right now:**
Same approach using `RUN_DAILY_UPDATE_ON_START: "true"`.

**Disable the nyaa fallback:**
Set `NYAA_ENABLED: "false"` in the configmap and restart.
