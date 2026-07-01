<p align="center">
  <img src="https://raw.githubusercontent.com/argyle-labs/jellyfin/main/assets/icon-256.png" width="120" alt="jellyfin" />
</p>

<p align="center">
  <a href="https://github.com/argyle-labs/jellyfin/actions/workflows/ci.yml"><img src="https://github.com/argyle-labs/jellyfin/actions/workflows/ci.yml/badge.svg" alt="CI" /></a>
  <a href="https://github.com/argyle-labs/jellyfin/actions/workflows/build.yml"><img src="https://github.com/argyle-labs/jellyfin/actions/workflows/build.yml/badge.svg" alt="Build and Push" /></a>
  <a href="https://github.com/argyle-labs/jellyfin/actions/workflows/release.yml"><img src="https://github.com/argyle-labs/jellyfin/actions/workflows/release.yml/badge.svg" alt="Release" /></a>
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
on `debian:12-slim`) runs `network_mode: host` on **:8096**
(`http://<host>:8096`).

```sh
cp examples/docker-compose.yml compose.yml
# edit the media mount + /opt/jellyfin paths, then:
docker compose up -d          # or: podman compose up -d
```

**One implementation, a few options.** [`examples/docker-compose.yml`](examples/docker-compose.yml)
is the whole deployment; GPU and transcode-scratch are independent options you
mix and match (all shown inline as comments), not separate setups:

- **GPU** — Intel/AMD via `/dev/dri` (default) **or** NVIDIA via the `nvidia`
  runtime (needs `nvidia-container-toolkit`).
- **Transcode scratch** — a disk path under `/cache` (default) **or** `tmpfs`
  (RAM), independent of the GPU choice.

**Not tied to our image.** `ghcr.io/argyle-labs/jellyfin` is a convenience build —
swap `image:` for any equivalent. [`examples/docker-compose.upstream.yml`](examples/docker-compose.upstream.yml)
is the same deployment on the official image:

| Image | Notes |
|---|---|
| `ghcr.io/argyle-labs/jellyfin` | this repo's slim build (`Dockerfile`); Intel VAAPI ready |
| `jellyfin/jellyfin` | official upstream image ([`examples/docker-compose.upstream.yml`](examples/docker-compose.upstream.yml)) |
| `lscr.io/linuxserver/jellyfin` | LinuxServer.io build (uses `PUID`/`PGID`, `/config` layout) |

Or build your own: `docker build -t jellyfin .`

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

Install from **Community Applications** (the *Apps* tab) — search **Jellyfin**
and add the template; it wires up the web UI, `/config`, and media shares for you.
Add `/dev/dri` (Settings → Docker, or the template's extra device) for Intel/AMD
hardware transcoding. To use this repo's image instead, set the template's
*Repository* to `ghcr.io/argyle-labs/jellyfin`. (Manual fallback: *Docker → Add
Container* with that image, port `8096`, `/config` + `/cache`, media read-only.)

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
- `examples/` — standalone compose: our image (`docker-compose.yml`) + the upstream image (`docker-compose.upstream.yml`), GPU/tmpfs shown inline as options.
- `lxc/` — `provision.sh` one-command Proxmox LXC + `jellyfin.conf.example` + VAAPI shim.
- `docs/` — [deploy-lxc.md](docs/deploy-lxc.md), the worked standalone LXC guide.
- `specs/`, `tests/` — API specs + tests.
- `assets/` — plugin icon.
