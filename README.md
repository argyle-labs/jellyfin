<p align="center">
  <img src="assets/icon-256.png" width="120" alt="jellyfin" />
</p>

# jellyfin

Self-hosted **[Jellyfin](https://jellyfin.org/)** media server — free-software
streaming for movies, TV, music, and books — packaged for hardware transcoding
(Intel QSV / AMD VAAPI / NVIDIA NVENC), plus a first-party
[orca](https://github.com/argyle-labs/orca) plugin for lifecycle and diagnostics.

This repo is **self-contained**: it builds its own slim image, ships ready-to-run
compose examples for each GPU, and a one-command Proxmox LXC provisioner — so you
can run Jellyfin **without orca** on docker, podman, an LXC, a VM, or Unraid.

---

## Run it without orca

### Docker / Podman

The image (`ghcr.io/argyle-labs/jellyfin`, built from [`Dockerfile`](Dockerfile)
on `debian:12-slim`) runs `network_mode: host` with `/dev/dri` passed through for
Intel/AMD VAAPI. Pick the example that matches your hardware:

| Example | For |
|---|---|
| [`examples/docker-compose.basic.yml`](examples/docker-compose.basic.yml) | Intel / AMD iGPU (VAAPI, `/dev/dri`) |
| [`examples/docker-compose.nvidia.yml`](examples/docker-compose.nvidia.yml) | NVIDIA (NVENC, needs `nvidia-container-toolkit`) |
| [`examples/docker-compose.tmpfs-transcode.yml`](examples/docker-compose.tmpfs-transcode.yml) | RAM-backed transcode scratch |
| [`examples/docker-compose.dockge.yml`](examples/docker-compose.dockge.yml) | Managed via Dockge (with healthcheck) |
| [`examples/docker-compose.yml`](examples/docker-compose.yml) | Plain upstream `jellyfin/jellyfin` image |

```sh
cp examples/docker-compose.basic.yml compose.yml
# edit the media mount + /opt/jellyfin paths, then:
docker compose up -d          # or: podman compose up -d
```

Jellyfin listens on **:8096**. Podman uses the same files (`podman compose up -d`).
Prefer the self-built image? `docker build -t jellyfin .` and point the compose
`image:` at it.

### LXC (Proxmox)

One command on the Proxmox host — no clone needed:

```sh
bash <(curl -fsSL https://raw.githubusercontent.com/argyle-labs/jellyfin/main/lxc/provision.sh) <vmid>
```

It builds a privileged Debian LXC with `/dev/dri` passthrough. For the full
manual walkthrough (GPU passthrough, NFS media mounts, QSV verification, nightly
backup, failover), see **[docs/deploy-lxc.md](docs/deploy-lxc.md)**; a sample
container config is in [`lxc/jellyfin.conf.example`](lxc/jellyfin.conf.example).

### VM / bare metal

Install Jellyfin from the upstream Debian repo on the guest (same steps as the
LXC guide's *Install Jellyfin* section), or run the container image inside the
VM. Pass through the GPU (`/dev/dri`, or an NVIDIA card) for hardware transcode.

### Unraid

*Docker → Add Container* with image `ghcr.io/argyle-labs/jellyfin` (or
`jellyfin/jellyfin`), port `8096`, `/config` + `/cache` app volumes, your media
share read-only, and device `/dev/dri` for Intel/AMD transcoding.

### Dependencies

- **GPU (recommended)** for hardware transcoding: Intel iGPU (QSV/VAAPI) or AMD
  (VAAPI) via `/dev/dri`, or an NVIDIA card via `nvidia-container-toolkit`.
  Software transcode works without one but is CPU-heavy.
- **A media library** mounted into the container/host (often NFS), read-only.

### Backup & restore

Jellyfin's state is its config/cache dirs (`/config`, `/var/lib/jellyfin` on a
native install). Stop it, `tar` those directories, restore by extracting them
back. The LXC guide includes a ready-made nightly backup timer.

> With orca this is **`jellyfin.backup` / `jellyfin.restore`** — see below.

---

## With orca

Unlike the generic `service.*` backends, jellyfin ships its **own typed tool
surface**, identical across **CLI, MCP, and REST** (generated from one
`#[orca_tool]` declaration):

| Tool | What it does |
|---|---|
| `jellyfin.install` / `jellyfin.update` | provision / upgrade the server |
| `jellyfin.backup` / `jellyfin.restore` | config backup + restore |
| `jellyfin.server_info` | server name / version / OS |
| `jellyfin.libraries` | configured libraries + paths |
| `jellyfin.transcode_health` | classify active sessions; flag **software fallback** (HW accel not engaging) |
| `jellyfin.memory_guard` | guard against runaway memory use |

```sh
orca jellyfin transcode_health --endpoint media   # is hardware transcode actually engaging?
```

## Layout

- `src/` — the orca plugin (the `jellyfin.*` tools above).
- `Dockerfile` + `scripts/` — build the slim image (`install`/`entrypoint`/`backup`/`restore`/`configure`).
- `examples/` — per-GPU compose files (Intel, NVIDIA, tmpfs, Dockge, upstream).
- `lxc/` — `provision.sh` one-command Proxmox LXC + `jellyfin.conf.example` + VAAPI shim.
- `docs/` — [deploy-lxc.md](docs/deploy-lxc.md), the worked standalone LXC guide.
- `specs/`, `tests/` — API specs + tests.
- `assets/` — plugin icon.
