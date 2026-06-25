# Ubuntu VAAPI shim — not required for Jellyfin

Unlike Plex, **Jellyfin does not need a glibc symbol shim** for hardware
transcoding.

Plex ships a musl-linked transcoder (`Plex Transcoder`) that uses an old
`gcompat` shim and cannot resolve modern glibc / C23 symbols when it `dlopen()`s
the system Intel iHD VAAPI driver — so Plex needs an `LD_PRELOAD` shim. Jellyfin
ships `jellyfin-ffmpeg`, which is **glibc-native and statically bundles its own
VAAPI/OpenCL stack**, so the driver loads cleanly on Debian 12 (and Ubuntu) with
no preload shim.

If VAAPI fails to initialise in a Jellyfin install, the cause is almost always
one of:

1. The `jellyfin` user is not in the GPU device group — see
   [`../provision.sh`](../provision.sh) / [`../../scripts/configure.sh`](../../scripts/configure.sh),
   which add it automatically.
2. `/dev/dri/renderD128` is not passed through to the container — see
   [`../jellyfin.conf.example`](../jellyfin.conf.example).
3. The wrong `LIBVA_DRIVER_NAME` — leave it `auto` and let the entrypoint probe.

Verify with:

```sh
/usr/lib/jellyfin-ffmpeg/vainfo --display drm --device /dev/dri/renderD128
```

This directory is kept only to mirror the Plex repo layout and to document the
difference; there is no `shim.c` to build.
