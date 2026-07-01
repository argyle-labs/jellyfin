# Jellyfin on a Proxmox LXC (Intel QSV hardware transcoding)

A worked, standalone deployment: Jellyfin in a **privileged Debian 12 LXC** on
Proxmox with Intel QuickSync (`/dev/dri`) passthrough and NFS-mounted media.
Privileged is required for `/dev/dri` access. Nothing here needs orca.

> Placeholders: `<proxmox-host>` = your Proxmox node, `<nas>` = your NAS/NFS
> server, `<ip>` = a LAN address. Pick the CT ID with `pvesh get /cluster/nextid`
> (shown here as `<CTID>`); never hard-code one.

- **Port**: 8096
- **Type**: Proxmox LXC — Debian 12 minimal, **privileged** (for `/dev/dri`)
- **GPU**: Intel QSV via `/dev/dri` passthrough

Design goal: the smallest possible LXC — Debian 12 minimal (~130 MB base), only
the packages Jellyfin + NFS + Intel VA-API need.

---

## Step 1 — Create the LXC

Proxmox UI → `<proxmox-host>` → Create CT:

| Field | Value |
|-------|-------|
| CT ID | `<CTID>` (`pvesh get /cluster/nextid`) |
| Hostname | jellyfin |
| Template | Debian 12 standard |
| Disk | 16 GB (local-lvm) — metadata only, no media stored here |
| CPU | 2 cores (QSV does the heavy lifting; 4 if needed) |
| RAM | 2048 MB |
| Swap | 512 MB |
| Network | vmbr0, DHCP (set a static lease after first boot) |
| **Unprivileged** | **No — must be privileged for /dev/dri** |

Or via CLI on `<proxmox-host>`:

```bash
pveam available | grep debian-12   # find the current template
pct create "$(pvesh get /cluster/nextid)" \
  local:vztmpl/debian-12-standard_12.7-1_amd64.tar.zst \
  --hostname jellyfin \
  --storage local-lvm \
  --rootfs local-lvm:16 \
  --cores 2 --memory 2048 --swap 512 \
  --net0 name=eth0,bridge=vmbr0,ip=dhcp \
  --unprivileged 0
```

## Step 2 — GPU passthrough + NFS bind mounts

Stop the LXC, then edit `/etc/pve/lxc/<CTID>.conf` on `<proxmox-host>`:

```ini
# Intel iGPU passthrough
dev0: /dev/dri/card0,gid=44
dev1: /dev/dri/renderD128,gid=44

# NFS bind mounts (host must have these mounted, e.g. via fstab)
mp0: /mnt/<nas>/backups/jellyfin,mp=/mnt/backups
mp1: /mnt/<nas>/data,mp=/mnt/data
```

Verify the host has the GPU and NFS available:

```bash
ls /dev/dri/          # must show card0 and renderD128
df -h | grep <nas>    # must show the NFS mounts
mkdir -p /mnt/<nas>/backups/jellyfin
```

## Step 3 — Minimal Debian

```bash
pct start <CTID>
pct enter <CTID>

apt-get update && apt-get upgrade -y
apt-get install -y --no-install-recommends \
  curl gnupg nfs-common intel-media-va-driver-non-free vainfo
apt-get remove --purge -y rsyslog cron at logrotate vim-tiny nano
apt-get autoremove --purge -y && apt-get clean && rm -rf /var/lib/apt/lists/*
```

> `intel-media-va-driver-non-free` is the iHD driver for 8th-gen+ Intel (needed
> for QSV). If it's not found, enable non-free first (`add-apt-repository non-free`).

## Step 4 — Install Jellyfin

```bash
install -d /etc/apt/keyrings
curl -fsSL https://repo.jellyfin.org/jellyfin_team.gpg.key \
  | gpg --dearmor -o /etc/apt/keyrings/jellyfin.gpg
cat > /etc/apt/sources.list.d/jellyfin.list << 'EOF'
deb [arch=amd64 signed-by=/etc/apt/keyrings/jellyfin.gpg] https://repo.jellyfin.org/debian bookworm main
EOF
apt-get update && apt-get install -y jellyfin
systemctl enable --now jellyfin
```

## Step 5 — Verify GPU access

```bash
ls /dev/dri/     # card0 and renderD128 must appear
vainfo           # should show the Intel iHD driver with H264/HEVC/AV1 profiles
```

If `vainfo` fails on permissions, the service user needs the `render` group (gid 44):

```bash
usermod -aG render jellyfin && systemctl restart jellyfin
```

## Step 6 — Static IP + first-run

Set a static DHCP lease (`<ip>`) for the LXC's MAC (`ip link show eth0`), then
open **http://<ip>:8096**, create the admin account, and add libraries from the
`/mnt/data` bind mount (e.g. `/mnt/data/media/{movies,tv,music}`).

## Step 7 — Hardware transcoding

Jellyfin → **Dashboard → Playback → Transcoding**: acceleration **Intel QuickSync
(QSV)**, enable hardware encoding, enable VPP tone mapping if available.

## Step 8 — Nightly config backup

Jellyfin's database/config lives in `/var/lib/jellyfin/`:

```bash
cat > /usr/local/bin/backup-jellyfin.sh << 'EOF'
#!/bin/sh
set -e
DEST=/mnt/backups; DATE=$(date +%Y%m%d_%H%M%S)
systemctl stop jellyfin
tar czf "$DEST/jellyfin_${DATE}.tar.gz" -C /var/lib jellyfin
systemctl start jellyfin
ls -dt "$DEST"/jellyfin_*.tar.gz | tail -n +8 | xargs -r rm -f
EOF
chmod +x /usr/local/bin/backup-jellyfin.sh
```

Schedule with a systemd timer (`OnCalendar=*-*-* 04:00:00`, `Persistent=true`)
rather than cron on minimal Debian.

---

## Resource notes

With QSV active, 2 cores / 2 GB RAM handles multiple simultaneous streams.
Without QSV, each 1080p software transcode can use 1–2 cores. The metadata
SQLite DB is small — 16 GB disk is plenty for most libraries.

## Failover to another node

1. Recreate the LXC on the other node with the same template + conf entries.
2. Restore: `tar xzf /mnt/backups/jellyfin_<latest>.tar.gz -C /var/lib/`.
3. Repoint the static DHCP lease to the new LXC's MAC → same IP.
4. Start Jellyfin — it picks up the same metadata and settings. (Any node with
   an Intel iGPU does QSV the same way.)

## Troubleshooting

**GPU not visible in the LXC** — on the host: `ls -la /dev/dri/` and confirm the
`dev0`/`dev1` lines in `/etc/pve/lxc/<CTID>.conf`.

**vainfo fails / QSV off** — `LIBVA_DRIVER_NAME=iHD vainfo`; if permission denied,
`usermod -aG render jellyfin && systemctl restart jellyfin`.

**SQLite lock (crash on login)** — stop Jellyfin, remove `*.db-shm`/`*.db-wal`
under `/var/lib/jellyfin`, start again.

**NFS bind mount missing in LXC** — the Proxmox host must mount the NFS paths
before the LXC starts (`df -h | grep <nas>`; `mount -a` if missing).
