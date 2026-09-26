#!/usr/bin/env bash
# Docker entrypoint for Jellyfin. Auto-detects the GPU VAAPI driver, fixes up
# the jellyfin user uid/gid + device group membership, and execs the server.
set -euo pipefail

CONFIG_DIR="${CONFIG_DIR:-/config}"
CACHE_DIR="${CACHE_DIR:-/cache}"

# Resolve VAAPI driver path for this architecture
ARCH=$(uname -m)
case "$ARCH" in
    aarch64) LIBVA_DRIVERS_PATH="${LIBVA_DRIVERS_PATH:-/usr/lib/aarch64-linux-gnu/dri}" ;;
    armv7l)  LIBVA_DRIVERS_PATH="${LIBVA_DRIVERS_PATH:-/usr/lib/arm-linux-gnueabihf/dri}" ;;
    *)       LIBVA_DRIVERS_PATH="${LIBVA_DRIVERS_PATH:-/usr/lib/x86_64-linux-gnu/dri}" ;;
esac
export LIBVA_DRIVERS_PATH

# Ensure jellyfin user/group match requested uid/gid. Capture the identity we are
# moving FROM — existing files under /config and /cache still carry it and have to
# be re-stamped further down, or the server cannot write its own database.
OLD_UID=""
OLD_GID=""
if ! getent group jellyfin > /dev/null 2>&1; then
    groupadd -g "${JELLYFIN_GID}" jellyfin
else
    # Move the GROUP itself. `usermod -g` needs the target gid to already exist,
    # so assuming some other group owns it silently fails the remap.
    OLD_GID=$(getent group jellyfin | cut -d: -f3)
    [[ "$OLD_GID" == "${JELLYFIN_GID}" ]] || groupmod -g "${JELLYFIN_GID}" jellyfin
fi
if ! getent passwd jellyfin > /dev/null 2>&1; then
    useradd -u "${JELLYFIN_UID}" -g "${JELLYFIN_GID}" -d /config -s /bin/bash jellyfin
else
    OLD_UID=$(id -u jellyfin)
    usermod -u "${JELLYFIN_UID}" -g "${JELLYFIN_GID}" jellyfin
fi

# Add jellyfin user to whatever groups own the GPU devices
shopt -s nullglob
for dev in /dev/dri/renderD128 /dev/dri/card0 /dev/nvidia*; do
    [[ -e "$dev" ]] || continue
    dev_gid=$(stat -c '%g' "$dev")
    if ! getent group "$dev_gid" > /dev/null 2>&1; then
        groupadd -g "$dev_gid" "gpu-${dev_gid}"
    fi
    usermod -aG "gpu-${dev_gid}" jellyfin 2>/dev/null || true
done
shopt -u nullglob

# Auto-detect GPU and select the VAAPI driver
detect_gpu() {
    if [[ -e /dev/nvidia0 ]]; then
        echo "nvidia"
        return
    fi
    if [[ -e /dev/dri/renderD128 ]]; then
        for driver in iHD radeonsi i965; do
            if LIBVA_DRIVER_NAME=$driver LIBVA_DRIVERS_PATH="${LIBVA_DRIVERS_PATH}" \
                vainfo --display drm --device /dev/dri/renderD128 > /dev/null 2>&1; then
                echo "$driver"
                return
            fi
        done
    fi
    echo "none"
}

if [[ "${LIBVA_DRIVER_NAME:-auto}" == "auto" ]]; then
    detected=$(detect_gpu)
    case "$detected" in
        nvidia)
            echo "[entrypoint] GPU: NVIDIA (NVENC/NVDEC)"
            unset LIBVA_DRIVER_NAME
            ;;
        none)
            echo "[entrypoint] GPU: none detected — software transcoding only"
            unset LIBVA_DRIVER_NAME
            ;;
        *)
            echo "[entrypoint] GPU: VAAPI driver=${detected}"
            export LIBVA_DRIVER_NAME="$detected"
            ;;
    esac
fi

mkdir -p "${CONFIG_DIR}" "${CACHE_DIR}"
chown jellyfin:jellyfin "${CONFIG_DIR}" "${CACHE_DIR}"

# Re-stamp state left behind by a PREVIOUS identity. Chowning just the two dirs
# above is not enough: everything already inside them keeps the old owner, so
# changing JELLYFIN_UID/GID on an existing install breaks the server on its own
# library.db with EACCES and the container crash-loops. Gated on the identity
# actually changing, so a normal start never pays for the walk — that gate is what
# makes this safe even when /config holds a large metadata tree.
#
# Media mounts are deliberately NOT touched: they arrive from outside, can hold
# millions of files, and their ownership belongs to whoever provisioned the share.
if [[ -n "$OLD_UID" && "$OLD_UID" != "${JELLYFIN_UID}" ]] \
   || [[ -n "$OLD_GID" && "$OLD_GID" != "${JELLYFIN_GID}" ]]; then
    echo "[entrypoint] identity remapped ${OLD_UID:-?}:${OLD_GID:-?} -> ${JELLYFIN_UID}:${JELLYFIN_GID} — re-stamping state"
    for state_dir in "${CONFIG_DIR}" "${CACHE_DIR}"; do
        [[ -d "$state_dir" ]] || continue
        if [[ -n "$OLD_UID" ]]; then
            find "$state_dir" -uid "$OLD_UID" -exec chown -h "${JELLYFIN_UID}" {} + 2>/dev/null || true
        fi
        if [[ -n "$OLD_GID" ]]; then
            find "$state_dir" -gid "$OLD_GID" -exec chgrp -h "${JELLYFIN_GID}" {} + 2>/dev/null || true
        fi
    done
fi

exec gosu jellyfin env \
    LIBVA_DRIVERS_PATH="${LIBVA_DRIVERS_PATH}" \
    ${LIBVA_DRIVER_NAME:+LIBVA_DRIVER_NAME="${LIBVA_DRIVER_NAME}"} \
    /usr/bin/jellyfin \
        --datadir "${CONFIG_DIR}" \
        --cachedir "${CACHE_DIR}" \
        --ffmpeg /usr/lib/jellyfin-ffmpeg/ffmpeg
