# Restarting the cluster + NAS after a physical move

Hardware:

| Host | IP | Role |
|---|---|---|
| january | 10.0.0.41 | Synology NAS — NFS server for all PVs, runs qBittorrent via docker-compose |
| bravostation | 10.0.0.11 | k3s control-plane (Alienware 13) |
| charliestation | 10.0.0.120 | k3s worker (ASUS ROG Zephyrus G14) |
| deltastation | 10.0.0.160 | k3s worker (Lenovo Legion 5) |

Everything (StorageClasses, PVs, registries.yaml on each node, app configmaps)
hardcodes these IPs. If the new location uses a different router/subnet,
reassign these same IPs (static config or DHCP reservations) to the same
physical machines before doing anything below.

## 1. Cable everything back up

Power + Ethernet for january and all three nodes, into the same switch/router
setup (or one issuing the same IPs — see above).

## 2. Power on january (NAS) first

Do not power on any k3s node before this — they mount NFS from it with `hard`
mounts, which hang indefinitely if the server isn't there yet.

```
ping 10.0.0.41
```

Wait until it responds, then confirm DSM is up: `http://10.0.0.41:5000`.

## 3. Confirm qBittorrent is running on january

It runs via docker-compose directly on the NAS, not in the cluster.

- Check `http://10.0.0.41:8090` loads the Web UI.
- If not, SSH into january and start it: `docker compose up -d` in the
  directory holding its compose file.

## 4. Power on bravostation (control-plane)

```
ssh bravostation "sudo systemctl status k3s"
```

Wait for `active (running)`, then:

```
ssh bravostation "kubectl get nodes"
```

## 5. Power on charliestation and deltastation (workers)

`k3s-agent` is enabled and starts automatically on boot. Confirm both rejoin:

```
ssh bravostation "kubectl get nodes -o wide"
```

All three should show `Ready`. bravostation will show
`Ready,SchedulingDisabled` — that's pre-existing (it was cordoned before the
move), not a problem.

## 6. Confirm workloads came back healthy

```
ssh bravostation "kubectl get pods -A -o wide"
```

Check `comics/comic-retrieval`, `torrent-allocator/torrent-allocator`, and
`registry/registry` are `Running` with low restart counts. If any is stuck
`ContainerCreating`, it's almost always the NFS mount — check the node can
reach january:

```
ssh <node> "showmount -e 10.0.0.41"
```

## Done

This is a cold power-cycle, not a redeploy — nothing was deleted, so no
`kubectl apply` or re-deploy step is needed. Pods reschedule automatically
once their node rejoins.
