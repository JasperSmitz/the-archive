#!/bin/sh
set -eu
# Railway mounts root-owned volumes. Its RAILWAY_RUN_UID=0 override is only for
# initializing this one directory; the application itself always drops to UID 10001.
if [ "$(id -u)" = 0 ]; then
    case "${1:-serve}" in
        serve|cleanup-orphans)
            [ "${IMAGE_STORAGE_DIR:-}" = /data/images ] || { echo 'Root volume initialization requires IMAGE_STORAGE_DIR=/data/images.' >&2; exit 1; }
            [ -d /data ] && [ ! -L /data ] && [ ! -L /data/images ] || { echo 'Expected a real mounted /data directory.' >&2; exit 1; }
            # Permit only the archive group to traverse the mount, even if its
            # initial mode was root-only. Backups elsewhere on the mount stay private.
            chown 0:10001 /data
            chmod 710 /data
            mkdir -p /data/images
            # Catalog files are generated flat files; do not follow links or chown the whole volume.
            find /data/images -maxdepth 1 -type f -exec chown 10001:10001 '{}' +
            chown 10001:10001 /data/images
            chmod 700 /data/images
            ;;
    esac
    exec gosu 10001:10001 /app/the-archive "$@"
fi
exec /app/the-archive "$@"
