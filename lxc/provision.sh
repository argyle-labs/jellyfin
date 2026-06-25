#!/usr/bin/env bash
# Creates and configures a Jellyfin LXC on Proxmox VE.
# Run on the Proxmox host as root — no git clone required.
#
# Usage:
#   bash <(curl -fsSL https://raw.githubusercontent.com/argyle-labs/jellyfin/main/lxc/provision.sh) <vmid> [options]
#
# Options:
#   --hostname NAME         LXC hostname (default: jellyfin)
#   --storage POOL          Proxmox storage pool for rootfs (default: local-lvm)
#   --disk SIZE             Root disk size (default: 16G)
#   --memory MB             RAM in MB (default: 4096 — right-sized for Intel transcode)
#   --cores N               CPU cores (default: 4)
#   --bridge BRIDGE         Network bridge (default: vmbr0)
#   --ip IP/CIDR            Static IP with prefix (e.g. <ip>/24)
#   --gw GATEWAY            Default gateway IP
#   --media PATH            Host path to media dir (mounted read-only at /mnt/media)
#   --config PATH           Host path for Jellyfin config (mounted at /config)
#   --jellyfin-version VER  Jellyfin version to install (default: latest)
#   --branch BRANCH         Repo branch to pull scripts from (default: main)
#   --render-gid GID        GID of /dev/dri/renderD128 on the host (default: 44)
#   --card-gid GID          GID of /dev/dri/card0 on the host (default: 44)
#   --no-gpu                Skip GPU passthrough (minimal/software-transcode setup)
#
# Recommended (Intel iHD/i965 or AMD radeonsi hardware transcode):
#   4 cores / 4096 MB RAM / shm 4g — NOT 16 GB. Jellyfin's transcode working set
#   lives in /cache + shm; the rootfs and RAM ceiling here are deliberately
#   modest. Oversizing was a past incident, not a requirement.
set -euo pipefail

VMID="${1:?Usage: $0 <vmid> [options]}"
shift

HOSTNAME="jellyfin"
STORAGE="local-lvm"
DISK="16G"
MEMORY="4096"
CORES="4"
BRIDGE="vmbr0"
IP=""
GW=""
MEDIA_PATH=""
CONFIG_PATH="/opt/jellyfin/config"
JELLYFIN_VERSION="latest"
BRANCH="main"
RENDER_GID="44"
CARD_GID="44"
NO_GPU=0

while [[ $# -gt 0 ]]; do
    case "$1" in
        --hostname)         HOSTNAME="$2";          shift 2 ;;
        --storage)          STORAGE="$2";           shift 2 ;;
        --disk)             DISK="$2";              shift 2 ;;
        --memory)           MEMORY="$2";            shift 2 ;;
        --cores)            CORES="$2";             shift 2 ;;
        --bridge)           BRIDGE="$2";            shift 2 ;;
        --ip)               IP="$2";                shift 2 ;;
        --gw)               GW="$2";                shift 2 ;;
        --media)            MEDIA_PATH="$2";        shift 2 ;;
        --config)           CONFIG_PATH="$2";       shift 2 ;;
        --jellyfin-version) JELLYFIN_VERSION="$2";  shift 2 ;;
        --branch)           BRANCH="$2";            shift 2 ;;
        --render-gid)       RENDER_GID="$2";        shift 2 ;;
        --card-gid)         CARD_GID="$2";          shift 2 ;;
        --no-gpu)           NO_GPU=1;               shift ;;
        *) echo "Unknown option: $1" >&2; exit 1 ;;
    esac
done

REPO_RAW="https://raw.githubusercontent.com/argyle-labs/jellyfin/${BRANCH}"

# ── Template ──────────────────────────────────────────────────────────────────
TEMPLATE_STORE="local"

# Find the newest Debian 12 standard template — prefer already-downloaded
TEMPLATE=$(sudo pveam list "$TEMPLATE_STORE" 2>/dev/null \
    | awk '{print $1}' \
    | sed 's|.*vztmpl/||' \
    | grep '^debian-12-standard' \
    | sort -V | tail -1)

if [[ -z "$TEMPLATE" ]]; then
    echo "[provision] No local Debian 12 template found, downloading..."
    sudo pveam update
    TEMPLATE=$(sudo pveam available --section system 2>/dev/null \
        | awk '{print $2}' \
        | grep '^debian-12-standard' \
        | sort -V | tail -1)
    if [[ -z "$TEMPLATE" ]]; then
        echo "[provision] ERROR: No debian-12-standard template available." >&2
        exit 1
    fi
    sudo pveam download "$TEMPLATE_STORE" "$TEMPLATE"
fi

echo "[provision] Using template: ${TEMPLATE}"

# ── Network ───────────────────────────────────────────────────────────────────
NET_ARGS="name=eth0,bridge=${BRIDGE},firewall=1"
if [[ -n "$IP" ]]; then
    NET_ARGS="${NET_ARGS},ip=${IP}"
    [[ -n "$GW" ]] && NET_ARGS="${NET_ARGS},gw=${GW}"
else
    NET_ARGS="${NET_ARGS},ip=dhcp"
fi

# ── Create container ──────────────────────────────────────────────────────────
echo "[provision] Creating LXC ${VMID} (${HOSTNAME})..."
# Privileged (--unprivileged 0) is required for GPU device passthrough in LXC
sudo pct create "$VMID" "${TEMPLATE_STORE}:vztmpl/${TEMPLATE}" \
    --hostname "$HOSTNAME" \
    --storage "$STORAGE" \
    --rootfs "${STORAGE}:${DISK}" \
    --memory "$MEMORY" \
    --cores "$CORES" \
    --net0 "$NET_ARGS" \
    --ostype debian \
    --unprivileged 0 \
    --start 0

# ── LXC config extras ─────────────────────────────────────────────────────────
{
    echo "lxc.apparmor.profile: unconfined"
    echo "lxc.seccomp.profile:"
} | sudo tee -a "/etc/pve/lxc/${VMID}.conf" > /dev/null

if [[ "$NO_GPU" -eq 0 ]]; then
    {
        echo "lxc.mount.entry: tmpfs dev/shm tmpfs nodev,nosuid,size=4g,mode=1777,create=dir 0 0"
        echo "dev0: /dev/dri/renderD128,gid=${RENDER_GID}"
        echo "dev1: /dev/dri/card0,gid=${CARD_GID}"
    } | sudo tee -a "/etc/pve/lxc/${VMID}.conf" > /dev/null
else
    echo "lxc.mount.entry: tmpfs dev/shm tmpfs nodev,nosuid,size=2g,mode=1777,create=dir 0 0" \
        | sudo tee -a "/etc/pve/lxc/${VMID}.conf" > /dev/null
fi

# ── Mount points (mp0 = media if provided, then config) ───────────────────────
MP=0
if [[ -n "$MEDIA_PATH" ]]; then
    echo "mp${MP}: ${MEDIA_PATH},mp=/mnt/media,ro=1" | sudo tee -a "/etc/pve/lxc/${VMID}.conf" > /dev/null
    MP=$((MP + 1))
fi
sudo mkdir -p "$CONFIG_PATH"
echo "mp${MP}: ${CONFIG_PATH},mp=/config" | sudo tee -a "/etc/pve/lxc/${VMID}.conf" > /dev/null

# ── Start and wait for network ────────────────────────────────────────────────
echo "[provision] Starting LXC ${VMID}..."
sudo pct start "$VMID"

echo "[provision] Waiting for network..."
for i in $(seq 1 30); do
    if sudo pct exec "$VMID" -- curl -fsSL --max-time 3 https://repo.jellyfin.org > /dev/null 2>&1; then
        break
    fi
    sleep 2
done

# ── Install and configure Jellyfin (fetched from public repo) ─────────────────
echo "[provision] Fetching and running install.sh..."
sudo pct exec "$VMID" -- bash -c \
    "curl -fsSL '${REPO_RAW}/scripts/install.sh' | bash -s -- '${JELLYFIN_VERSION}'"

echo "[provision] Fetching and running configure.sh..."
sudo pct exec "$VMID" -- bash -c \
    "curl -fsSL '${REPO_RAW}/scripts/configure.sh' | bash"

# ── Done ──────────────────────────────────────────────────────────────────────
LXC_IP=$(sudo pct exec "$VMID" -- hostname -I 2>/dev/null | awk '{print $1}')
echo ""
echo "[provision] Done. LXC ${VMID} (${HOSTNAME}) is running Jellyfin."
echo "            http://${LXC_IP}:8096/web"
