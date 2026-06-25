#!/usr/bin/env bash
# Backs up Jellyfin config (the data dir). Regenerable cache/transcodes/logs are
# excluded. Mirrors the orca `jellyfin.backup` tool for shell-bootstrap use.
#
# Invocation (installed as /usr/local/bin/backup by install.sh):
#   LXC:    pct exec <vmid> -- backup [--output DIR]
#   Docker: docker exec jellyfin backup [--output DIR]
#   Host:   backup --container jellyfin --output /opt/jellyfin/backups
set -euo pipefail

OUTPUT_DIR=""
CONTAINER=""

while [[ $# -gt 0 ]]; do
    case "$1" in
        --output)     OUTPUT_DIR="$2";  shift 2 ;;
        --container)  CONTAINER="$2";   shift 2 ;;
        *) echo "Unknown option: $1" >&2; exit 1 ;;
    esac
done

HAS_SYSTEMCTL=0
command -v systemctl > /dev/null 2>&1 && systemctl is-system-running > /dev/null 2>&1 && HAS_SYSTEMCTL=1 || true

jf_stop() {
    if [[ -n "$CONTAINER" ]]; then
        echo "[backup] Stopping Docker container: ${CONTAINER}..."
        docker stop "$CONTAINER" 2>/dev/null || true
    elif [[ $HAS_SYSTEMCTL -eq 1 ]]; then
        echo "[backup] Stopping jellyfin (systemctl)..."
        systemctl stop jellyfin 2>/dev/null || true
    fi
}

jf_start() {
    if [[ -n "$CONTAINER" ]]; then
        docker start "$CONTAINER" 2>/dev/null || true
    elif [[ $HAS_SYSTEMCTL -eq 1 ]]; then
        systemctl start jellyfin 2>/dev/null || true
    fi
}

# ── Auto-detect Jellyfin data dir ─────────────────────────────────────────────
if [[ -n "$CONTAINER" ]]; then
    DATA_DIR=$(docker inspect "$CONTAINER" \
        --format '{{range .Mounts}}{{if eq .Destination "/config"}}{{.Source}}{{end}}{{end}}' 2>/dev/null || true)
    [[ -n "$DATA_DIR" ]] || { echo "[backup] Error: could not determine /config volume for '${CONTAINER}'" >&2; exit 1; }
elif [[ -d /config ]]; then
    DATA_DIR="/config"
elif [[ -d /var/lib/jellyfin ]]; then
    DATA_DIR="/var/lib/jellyfin"
else
    echo "[backup] Error: Jellyfin data dir not found. Use --container NAME for host-side Docker." >&2
    exit 1
fi

if [[ -z "$OUTPUT_DIR" ]]; then
    if [[ -d /mnt/backups ]]; then OUTPUT_DIR="/mnt/backups"; else OUTPUT_DIR="$(pwd)"; fi
fi
mkdir -p "$OUTPUT_DIR"

TIMESTAMP=$(date +%Y%m%d-%H%M%S)
OUT_FILE="${OUTPUT_DIR}/jellyfin-config-${TIMESTAMP}.tar.gz"
echo "[backup] Source: ${DATA_DIR}"
echo "[backup] Output: ${OUT_FILE}"

jf_stop
trap jf_start EXIT

tar -czf "$OUT_FILE" \
    --exclude=./cache \
    --exclude=./transcodes \
    --exclude=./log \
    -C "$DATA_DIR" .

SIZE=$(du -sh "$OUT_FILE" | cut -f1)
echo "[backup] Done. ${SIZE} → ${OUT_FILE}"
