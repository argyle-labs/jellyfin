FROM debian:12-slim

ARG JELLYFIN_VERSION=latest

ENV DEBIAN_FRONTEND=noninteractive \
    JELLYFIN_UID=1000 \
    JELLYFIN_GID=1000 \
    LIBVA_DRIVER_NAME=auto

COPY scripts/install.sh /tmp/install.sh
RUN chmod +x /tmp/install.sh && SKIP_SCRIPT_DOWNLOAD=1 /tmp/install.sh "${JELLYFIN_VERSION}" && rm /tmp/install.sh

COPY scripts/entrypoint.sh /entrypoint.sh
COPY scripts/backup.sh /usr/local/bin/backup
COPY scripts/restore.sh /usr/local/bin/restore
RUN chmod +x /entrypoint.sh /usr/local/bin/backup /usr/local/bin/restore

EXPOSE 8096

VOLUME ["/config", "/cache"]

HEALTHCHECK --interval=30s --timeout=10s --start-period=60s --retries=3 \
    CMD curl -fsSL http://localhost:8096/System/Info/Public > /dev/null || exit 1

ENTRYPOINT ["/entrypoint.sh"]
