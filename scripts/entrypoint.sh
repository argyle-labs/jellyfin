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

# Ensure jellyfin user/group match requested uid/gid
if ! getent group jellyfin > /dev/null 2>&1; then
    groupadd -g "${JELLYFIN_GID}" jellyfin
fi
if ! getent passwd jellyfin > /dev/null 2>&1; then
    useradd -u "${JELLYFIN_UID}" -g "${JELLYFIN_GID}" -d /config -s /bin/bash jellyfin
else
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

exec gosu jellyfin env \
    LIBVA_DRIVERS_PATH="${LIBVA_DRIVERS_PATH}" \
    ${LIBVA_DRIVER_NAME:+LIBVA_DRIVER_NAME="${LIBVA_DRIVER_NAME}"} \
    /usr/bin/jellyfin \
        --datadir "${CONFIG_DIR}" \
        --cachedir "${CACHE_DIR}" \
        --ffmpeg /usr/lib/jellyfin-ffmpeg/ffmpeg
