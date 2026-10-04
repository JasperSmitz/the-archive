#!/bin/sh
# libpq reads credentials from PGPASSFILE/PGSERVICE; no passwords in arguments.
set -eu
umask 077
: "${PGHOST:?Set PGHOST for the source database}"
: "${PGUSER:?Set PGUSER for the source database}"
: "${PGDATABASE:?Set PGDATABASE to a database name, not a credential URL}"
: "${IMAGE_STORAGE_DIR:?Set the source IMAGE_STORAGE_DIR}"
[ "${ARCHIVE_WRITES_STOPPED:-}" = yes ] || { echo 'Stop ALL application writers, then set ARCHIVE_WRITES_STOPPED=yes.' >&2; exit 1; }
[ "$#" = 1 ] || { echo 'Usage: scripts/backup.sh NEW_BACKUP_DIRECTORY' >&2; exit 1; }
case "$PGDATABASE" in *://*) echo 'PGDATABASE must be a database name, not a URL.' >&2; exit 1;; esac
[ -d "$IMAGE_STORAGE_DIR" ] || { echo 'Image directory does not exist.' >&2; exit 1; }
[ ! -e "$1" ] || { echo 'Backup destination must not already exist.' >&2; exit 1; }
mkdir -m 700 "$1"
pg_dump --format=custom --no-owner --no-privileges --file="$1/archive.dump"
tar -czf "$1/images.tar.gz" -C "$IMAGE_STORAGE_DIR" .
(cd "$1" && sha256sum archive.dump images.tar.gz > SHA256SUMS)
echo 'Coordinated backup complete. Store this private bundle away from the live volume.'
