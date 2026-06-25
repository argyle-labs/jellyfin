# jellyfin

Self-hosted Jellyfin media server with automatic GPU detection and hardware
transcoding — plus a first-class [orca](https://github.com/argyle-labs/orca)
plugin that owns the full lifecycle (install, update, backup/restore) and live
transcode-session diagnosis.

Built from `debian:12-slim`. Supports native LXC, Docker, Podman, and Dockge.
Works with Intel, AMD, and NVIDIA GPUs on `amd64` and `arm64`. Jellyfin serves on
`:8096`, config at `/config`, cache at `/cache`.

## Two halves of one repo

| Half | What it is |
|---|---|
| **Deploy assets** (`Dockerfile`, `compose.yml`, `lxc/`, `scripts/`, `examples/`) | Curl-bootstrappable install/update/backup payload — no git clone needed. |
| **orca plugin** (`src/`, `Cargo.toml`, `build.rs`, `specs/`) | A Rust orca plugin whose only orca dependency is `plugin-toolkit`. Exposes the lifecycle + diagnosis as `#[orca_tool]`s. |

## Deployment Paths

| Path | Use case |
|---|---|
| [Native LXC](#proxmox-lxc-native) | Proxmox — preferred, no Docker overhead |
| [Docker / Compose](#docker--compose) | Any Linux host with Docker |
| [orca tools](#orca-plugin) | Drive install/update/backup/restore + diagnosis through orca |

## Specs

### Minimal (software transcode only)

| Resource | Value |
|---|---|
| CPU | 2 cores |
| RAM | 2 GB |
| Disk | 16 GB (rootfs) |
| GPU | none required |
| shm | 2 GB |

Software transcode works but is CPU-bound. 4K content will max out cores.

### Recommended (hardware transcode)

| Resource | Value |
|---|---|
| CPU | 4 cores |
| RAM | **4 GB** |
| Disk | 16 GB (rootfs) |
| GPU | Intel iHD Gen8+ (UHD 600+) or AMD GCN+ |
| shm | 4 GB |

Right-sized for Intel GPU transcoding: **4 cores / 4 GB RAM / shm 4g** — **not
16 GB.** Oversizing was a past incident, not a requirement. Jellyfin's transcode
working set lives in `/cache` + shm, not resident RAM.

## GPU Support

| GPU | Generations | Driver | Notes |
|---|---|---|---|
| Intel iHD | Gen8+ (Broadwell → Arrow Lake, UHD 600+) | `iHD` | amd64 only |
| Intel i965 | Gen4–9 (HD 2000–6000, Haswell, Skylake) | `i965` | Open source; amd64 only |
| AMD | GCN+ (RX 400+), RDNA 1/2/3 | `radeonsi` | Via Mesa |
| NVIDIA | GTX 900+ / RTX | NVENC/NVDEC | Requires `nvidia-container-toolkit` on host |

`LIBVA_DRIVER_NAME=auto` (the default) probes available hardware and selects the
right driver automatically.

> **No glibc shim needed.** Unlike Plex, Jellyfin ships `jellyfin-ffmpeg` which
> is glibc-native — there is no `LD_PRELOAD` VAAPI shim. See
> [`lxc/ubuntu-vaapi-shim/README.md`](lxc/ubuntu-vaapi-shim/README.md).

---

## Proxmox LXC (native)

The preferred deployment on Proxmox: plain Debian 12 LXC with Jellyfin installed
directly — no Docker.

### Automated provisioning

Run on the Proxmox host as root — no git clone needed:

```sh
# Minimal — software transcode only, 2 cores / 2 GB RAM
bash <(curl -fsSL https://raw.githubusercontent.com/argyle-labs/jellyfin/main/lxc/provision.sh) 113 \
  --hostname jellyfin \
  --disk 16G \
  --memory 2048 \
  --cores 2 \
  --no-gpu \
  --config /opt/jellyfin/config \
  --media /mnt/<pool>/data

# Recommended — Intel/AMD GPU, hardware transcode (4 cores / 4 GB)
bash <(curl -fsSL https://raw.githubusercontent.com/argyle-labs/jellyfin/main/lxc/provision.sh) 113 \
  --hostname jellyfin \
  --memory 4096 \
  --cores 4 \
  --config /opt/jellyfin/config \
  --media /mnt/<pool>/data

# Pinned Jellyfin version
bash <(curl -fsSL https://raw.githubusercontent.com/argyle-labs/jellyfin/main/lxc/provision.sh) 113 \
  --hostname jellyfin \
  --jellyfin-version 10.9.11 \
  --config /opt/jellyfin/config \
  --media /mnt/<pool>/data
```

The script resolves the latest Debian 12 template, creates the LXC, configures
GPU passthrough, starts it, downloads and runs `install.sh` + `configure.sh`
from this repo, and prints the Jellyfin URL.

GPU device GIDs default to 44 (standard on Debian/Ubuntu Proxmox hosts). Override
with `--render-gid` / `--card-gid`, or skip with `--no-gpu`.

### Manual install

Create and start a Debian 12 LXC using `lxc/jellyfin.conf.example` as a
reference, then inside the LXC:

```sh
curl -fsSL https://raw.githubusercontent.com/argyle-labs/jellyfin/main/scripts/install.sh | bash
curl -fsSL https://raw.githubusercontent.com/argyle-labs/jellyfin/main/scripts/configure.sh | bash
```

### Verify GPU (LXC)

```sh
pct exec <vmid> -- /usr/lib/jellyfin-ffmpeg/vainfo --display drm --device /dev/dri/renderD128
```

---

## Docker / Compose

### Quick start

```sh
docker run -d \
  --name jellyfin \
  --network=host \
  --shm-size=4g \
  --restart=unless-stopped \
  -e JELLYFIN_UID=$(id -u) \
  -e JELLYFIN_GID=$(id -g) \
  -v /etc/localtime:/etc/localtime:ro \
  -v /opt/jellyfin/config:/config \
  -v /opt/jellyfin/cache:/cache \
  -v /mnt/media:/mnt/media:ro \
  --device /dev/dri/renderD128 \
  --device /dev/dri/card0 \
  ghcr.io/argyle-labs/jellyfin:latest
```

### Compose examples

See the [`examples/`](examples/) directory:

| File | Description |
|---|---|
| `docker-compose.basic.yml` | Minimal setup — Intel/AMD GPU, auto-detect |
| `docker-compose.dockge.yml` | Dockge-managed with healthcheck |
| `docker-compose.tmpfs-transcode.yml` | RAM-backed `/cache` (fastest, no disk wear) |
| `docker-compose.nvidia.yml` | NVIDIA GPU via nvidia-container-toolkit |

```sh
cp examples/docker-compose.dockge.yml compose.yml
# edit: set JELLYFIN_UID, JELLYFIN_GID, volume paths
docker compose up -d
```

### Environment variables

| Variable | Default | Description |
|---|---|---|
| `JELLYFIN_UID` | `1000` | UID for the jellyfin process |
| `JELLYFIN_GID` | `1000` | GID for the jellyfin process |
| `LIBVA_DRIVER_NAME` | `auto` | `auto`, `iHD`, `i965`, `radeonsi` — auto-detected at startup |
| `CONFIG_DIR` | `/config` | Jellyfin data dir — **must be persistent** |
| `CACHE_DIR` | `/cache` | Cache + transcode working dir — can be tmpfs |

### Volumes

| Mount | Description |
|---|---|
| `/etc/localtime` | Mount from host (`ro`) — sets container timezone |
| `/config` | Jellyfin library, database, preferences — **must be persistent** |
| `/cache` | Cache + transcode working dir — can be tmpfs |
| `/mnt/media` | Your media library — mount any path here (read-only recommended) |

### Verify GPU (Docker)

```sh
docker exec jellyfin /usr/lib/jellyfin-ffmpeg/vainfo --display drm --device /dev/dri/renderD128
```

---

## orca plugin

The crate in `src/` is an orca plugin. Its **only** orca dependency is
`plugin-toolkit` (pinned to an orca rc tag; a committed `.cargo/config.toml`
`[patch]` overrides it to a local `../orca` checkout for development). All
genuinely-external deps (progenitor client, reqwest, serde, chrono, …) are the
plugin's own, exactly as a third-party plugin would carry.

### Tool surface

| Tool | Purpose |
|---|---|
| `jellyfin.{list,detail,create,update,delete}` | Endpoint registry CRUD (generated by `endpoint_resource!`). |
| `jellyfin.server_info` | Server name / version / OS from `/System/Info`. |
| `jellyfin.libraries` | Configured libraries from `/Library/VirtualFolders`. |
| `jellyfin.transcode_health` | **Core diagnosis** — per-session HW-vs-software transcode state. |
| `jellyfin.memory_guard` | **Self-heal** — detect transcode-memory pressure → restart → notify. |
| `jellyfin.install` | Provision an LXC or Compose deployment. |
| `jellyfin.update` | Channel-aware (`latest`/`rc`/`stable`) image/version bump. |
| `jellyfin.backup` | Tar the `/config` volume to a destination directory. |
| `jellyfin.restore` | Restore the `/config` volume from a tarball (`--from`). |

The `memory_guard` tool encodes the transcode-memory-exhaustion incident as a
detect → remediate → notify capability: a real liveness probe + transcode-load
count drives an optional `POST /System/Restart` that reaps the ffmpeg workers,
then re-probes and emits a notification.

### Build

```sh
git clone https://github.com/argyle-labs/jellyfin
cd jellyfin
# With an orca checkout at ../orca, the committed .cargo/config.toml patch
# resolves plugin-toolkit locally; otherwise it resolves from the pinned rc tag.
cargo build
cargo test
```

---

## Backup & Restore

Two equivalent paths — the orca tools and the shell scripts share the same
archive format (`jellyfin-config-YYYYMMDD-HHMMSS.tar.gz`, excluding the
regenerable `cache/`, `transcodes/`, and `log/` trees).

### orca tools

```sh
# Back up the /config volume to a directory
orca jellyfin backup --config-path /opt/jellyfin/config --destination /mnt/backups

# Restore from a specific tarball
orca jellyfin restore --from /mnt/backups/jellyfin-config-20260625-010000.tar.gz \
  --config-path /opt/jellyfin/config
```

### Shell scripts (`backup` / `restore`, installed at `/usr/local/bin`)

```sh
# LXC
pct exec <vmid> -- backup
pct exec <vmid> -- restore            # lists backups, restores latest
pct exec <vmid> -- restore --list

# Docker (inside the container)
docker exec jellyfin backup
docker exec jellyfin restore

# Host-side Docker — stops container, backs up /config volume, restarts
backup --container jellyfin --output /opt/jellyfin/backups
restore /opt/jellyfin/backups/jellyfin-config-20260625-010000.tar.gz --container jellyfin
```

---

## Tags

```
ghcr.io/argyle-labs/jellyfin:latest    # newest Jellyfin release
```

## Building Locally

```sh
docker build -t jellyfin-local .
# pin a specific Jellyfin version:
docker build --build-arg JELLYFIN_VERSION=10.9.11 -t jellyfin-local .
```
