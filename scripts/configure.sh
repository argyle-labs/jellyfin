#!/usr/bin/env bash
# Configures Jellyfin for a bare LXC/VM install.
# Run once after install.sh. Not used in Docker (entrypoint.sh handles Docker).
# Usage: configure.sh
set -euo pipefail

# Resolve VAAPI driver path
ARCH=$(uname -m)
case "$ARCH" in
    aarch64) LIBVA_DRIVERS_PATH="/usr/lib/aarch64-linux-gnu/dri" ;;
    armv7l)  LIBVA_DRIVERS_PATH="/usr/lib/arm-linux-gnueabihf/dri" ;;
    *)       LIBVA_DRIVERS_PATH="/usr/lib/x86_64-linux-gnu/dri" ;;
esac

detect_gpu() {
    if [[ -e /dev/dri/renderD128 ]]; then
        for driver in iHD radeonsi i965; do
            if LIBVA_DRIVER_NAME=$driver LIBVA_DRIVERS_PATH="$LIBVA_DRIVERS_PATH" \
                vainfo --display drm --device /dev/dri/renderD128 > /dev/null 2>&1; then
                echo "$driver"
                return
            fi
        done
    fi
    echo "none"
}

GPU_DRIVER=$(detect_gpu)
echo "[configure] GPU driver: ${GPU_DRIVER}"

# Add jellyfin to GPU device groups (the package creates the jellyfin user)
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

# Mount point /config (from the LXC provision) is Jellyfin's data dir. Point the
# service at it via a systemd drop-in + the package's default-data override.
mkdir -p /etc/systemd/system/jellyfin.service.d
{
    echo "[Service]"
    echo "Environment=LIBVA_DRIVERS_PATH=${LIBVA_DRIVERS_PATH}"
    if [[ "$GPU_DRIVER" != "none" ]]; then
        echo "Environment=LIBVA_DRIVER_NAME=${GPU_DRIVER}"
    fi
} > /etc/systemd/system/jellyfin.service.d/gpu.conf

# Repoint Jellyfin data dir to the /config mount if present.
if [[ -d /config ]]; then
    mkdir -p /etc/default
    {
        echo 'JELLYFIN_DATA_DIR="/config"'
        echo 'JELLYFIN_CACHE_DIR="/config/cache"'
    } > /etc/default/jellyfin
    chown -R jellyfin:jellyfin /config 2>/dev/null || true
fi

systemctl daemon-reload
systemctl enable --now jellyfin

LXC_IP=$(hostname -I | awk '{print $1}')
echo ""
echo "[configure] Done. Jellyfin is running at http://${LXC_IP}:8096/web"
echo "[configure] Verify GPU: vainfo --display drm --device /dev/dri/renderD128"
