#!/usr/bin/env bash
# Restores Jellyfin config from a backup created by backup.
# Installed as /usr/local/bin/restore by install.sh. Mirrors the orca
# `jellyfin.restore --from <tarball>` tool for shell-bootstrap use.
#
# Usage:
#   restore                        # list backups, restore latest
#   restore --list                 # list available backups and exit
#   restore <backup-file.tar.gz>   # restore specific file
#
# Options:
#   --container NAME   Docker container name (host-side invocation)
#   --force            Skip the 3-second abort window
set -euo pipefail

BACKUP_FILE=""
CONTAINER=""
FORCE=0
LIST_ONLY=0

if [[ $# -gt 0 ]] && [[ "$1" != --* ]]; then
    BACKUP_FILE="$1"
    shift
fi

while [[ $# -gt 0 ]]; do
    case "$1" in
        --container)  CONTAINER="$2";  shift 2 ;;
        --force)      FORCE=1;         shift ;;
        --list)       LIST_ONLY=1;     shift ;;
        *) echo "Unknown option: $1" >&2; exit 1 ;;
    esac
done

# ── Auto-detect Jellyfin data dir ─────────────────────────────────────────────
if [[ -n "$CONTAINER" ]]; then
    DATA_DIR=$(docker inspect "$CONTAINER" \
        --format '{{range .Mounts}}{{if eq .Destination "/config"}}{{.Source}}{{end}}{{end}}' 2>/dev/null || true)
    [[ -n "$DATA_DIR" ]] || { echo "[restore] Error: could not determine /config volume for '${CONTAINER}'" >&2; exit 1; }
elif [[ -d /config ]]; then
    DATA_DIR="/config"
elif [[ -d /var/lib/jellyfin ]]; then
    DATA_DIR="/var/lib/jellyfin"
else
    echo "[restore] Error: Jellyfin data dir not found. Use --container NAME for host-side Docker." >&2
    exit 1
fi

HAS_SYSTEMCTL=0
command -v systemctl > /dev/null 2>&1 && systemctl is-system-running > /dev/null 2>&1 && HAS_SYSTEMCTL=1 || true

jf_stop() {
    if [[ -n "$CONTAINER" ]]; then
        docker stop "$CONTAINER" 2>/dev/null || true
    elif [[ $HAS_SYSTEMCTL -eq 1 ]]; then
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

find_backup_dir() {
    if [[ -n "${BACKUP_DIR:-}" ]] && [[ -d "$BACKUP_DIR" ]]; then echo "$BACKUP_DIR"
    elif [[ -d /mnt/backups ]]; then echo "/mnt/backups"
    elif [[ -d /backups ]]; then echo "/backups"
    else echo "$(pwd)"; fi
}

BACKUP_SEARCH_DIR=$(find_backup_dir)

if [[ -z "$BACKUP_FILE" ]]; then
    mapfile -t BACKUPS < <(find "$BACKUP_SEARCH_DIR" -maxdepth 1 -name 'jellyfin-config-*.tar.gz' | sort -r)
    if [[ ${#BACKUPS[@]} -eq 0 ]]; then
        echo "[restore] No backups found in ${BACKUP_SEARCH_DIR}" >&2
        exit 1
    fi
    echo "[restore] Available backups in ${BACKUP_SEARCH_DIR}:"
    for i in "${!BACKUPS[@]}"; do echo "  [$i] ${BACKUPS[$i]##*/}"; done
    echo ""
    if [[ $LIST_ONLY -eq 1 ]]; then exit 0; fi
    BACKUP_FILE="${BACKUPS[0]}"
    echo "[restore] Using latest: ${BACKUP_FILE##*/}"
fi

[[ -f "$BACKUP_FILE" ]] || { echo "[restore] Error: backup file not found: $BACKUP_FILE" >&2; exit 1; }

if [[ $FORCE -eq 0 ]]; then
    echo "[restore] Restoring ${BACKUP_FILE##*/} in 3 seconds — Ctrl-C to abort"
    sleep 3
fi

jf_stop
trap jf_start EXIT

echo "[restore] Extracting to ${DATA_DIR}..."
tar -xzf "$BACKUP_FILE" -C "$DATA_DIR"
getent passwd jellyfin > /dev/null 2>&1 && chown -R jellyfin:jellyfin "$DATA_DIR" || true

echo "[restore] Done. Restored to: ${DATA_DIR}"
