<p align="center">
  <img src="assets/icon-256.png" width="120" alt="jellyfin" />
</p>

# jellyfin

Jellyfin is a free-software media server for streaming movies, TV, and music, with hardware transcoding.

A first-party [orca](https://github.com/argyle-labs/orca) plugin (service-backend).

This repo **ships a `compose.yml`** — run jellyfin **by hand, without orca** straight from it:

---

## Run it without orca

```sh
docker compose up -d
```

See [`compose.yml`](compose.yml) for the image, ports, volumes, and hardware/device mappings and `scripts/` for provisioning helpers. Upstream docs: <https://jellyfin.org/>.


### Backup & restore

Back up the config/data volume(s) above — that's the whole service state (stop the container first for a clean copy). Restore by putting them back and starting it.

> With orca this is **`service.backup` / `service.restore`** — location-agnostic (docker / podman / lxc / vm), one command regardless of where jellyfin runs. No per-service backup script.

## With orca

orca drives this plugin through the single generic `service.*` surface — no per-plugin tools:

```sh
orca service.deploy jellyfin      # render + launch on any supported runtime
orca service.status jellyfin      # health + rich diagnostics (typed payload)
orca service.backup jellyfin      # location-agnostic backup (tar; PBS on Proxmox)
orca service.configure jellyfin   # apply config via the upstream API
```

## Layout

- `src/` — the plugin (pure Rust): the `ServiceBackend` descriptor + `configure` / `status`.
- `compose.yml` — standalone deployment.
- `scripts/` — provisioning / lifecycle helpers.
- `assets/` — plugin icon.
