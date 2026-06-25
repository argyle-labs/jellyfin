#!/usr/bin/env bash
# Installs Jellyfin + GPU drivers on Debian 12.
# Works in Docker (called from Dockerfile) and bare LXC/VM.
# Usage: install.sh [jellyfin-version]   — defaults to "latest"
#
# Unlike Plex, Jellyfin's bundled ffmpeg (jellyfin-ffmpeg) is glibc-native, so
# no LD_PRELOAD glibc shim is needed for VAAPI hardware transcoding on Debian 12.
set -euo pipefail

JELLYFIN_VERSION="${1:-latest}"

export DEBIAN_FRONTEND=noninteractive

apt-get update
apt-get install -y --no-install-recommends \
    ca-certificates \
    curl \
    gnupg \
    tzdata \
    udev \
    ocl-icd-libopencl1 \
    mesa-va-drivers \
    gosu \
    vainfo

# Enable non-free repos for Intel drivers (idempotent).
# Debian 12 Docker image uses DEB822 format; Proxmox LXC templates use legacy sources.list.
if [ -f /etc/apt/sources.list.d/debian.sources ]; then
    if ! grep -q 'non-free' /etc/apt/sources.list.d/debian.sources; then
        sed -i 's/^Components: main$/Components: main contrib non-free non-free-firmware/' \
            /etc/apt/sources.list.d/debian.sources
    fi
elif [ -f /etc/apt/sources.list ]; then
    if ! grep -q 'non-free' /etc/apt/sources.list; then
        sed -i '/debian\.org\/debian/s/$/ contrib non-free non-free-firmware/' \
            /etc/apt/sources.list
    fi
fi

# Intel drivers are x86-only
ARCH=$(dpkg --print-architecture)
if [ "$ARCH" = "amd64" ]; then
    apt-get update
    apt-get install -y --no-install-recommends \
        intel-media-va-driver-non-free \
        i965-va-driver \
        intel-opencl-icd
fi

# ── Install Jellyfin from the official apt repository ─────────────────────────
DISTRO_CODENAME="bookworm"
curl -fsSL https://repo.jellyfin.org/jellyfin_team.gpg.key \
    | gpg --dearmor -o /usr/share/keyrings/jellyfin-archive-keyring.gpg
cat > /etc/apt/sources.list.d/jellyfin.sources <<SRC
Types: deb
URIs: https://repo.jellyfin.org/debian
Suites: ${DISTRO_CODENAME}
Components: main
Architectures: ${ARCH}
Signed-By: /usr/share/keyrings/jellyfin-archive-keyring.gpg
SRC

apt-get update
if [ "$JELLYFIN_VERSION" = "latest" ]; then
    apt-get install -y --no-install-recommends jellyfin-server jellyfin-ffmpeg7 jellyfin-web
else
    apt-get install -y --no-install-recommends \
        "jellyfin-server=${JELLYFIN_VERSION}" jellyfin-ffmpeg7 jellyfin-web
fi

apt-get clean
find /var/lib/apt/lists -type f -delete

# Install backup and restore commands.
# SKIP_SCRIPT_DOWNLOAD=1 when called from Dockerfile (Dockerfile COPYs them directly after this step).
if [[ "${SKIP_SCRIPT_DOWNLOAD:-0}" != "1" ]]; then
    REPO_RAW="${REPO_RAW:-https://raw.githubusercontent.com/argyle-labs/jellyfin/main}"
    curl -fsSL "${REPO_RAW}/scripts/backup.sh" -o /usr/local/bin/backup
    curl -fsSL "${REPO_RAW}/scripts/restore.sh" -o /usr/local/bin/restore
    chmod +x /usr/local/bin/backup /usr/local/bin/restore
fi
