#!/bin/sh
set -eu
umask 077
: "${PGHOST:?Set PGHOST for the isolated destination database}"
: "${PGUSER:?Set PGUSER for the isolated destination database}"
: "${PGDATABASE:?Set PGDATABASE to a database name, not a credential URL}"
: "${IMAGE_STORAGE_DIR:?Set an empty isolated destination IMAGE_STORAGE_DIR}"
[ "${ARCHIVE_WRITES_STOPPED:-}" = yes ] || { echo 'Stop ALL application writers, then set ARCHIVE_WRITES_STOPPED=yes.' >&2; exit 1; }
[ "$#" = 1 ] || { echo 'Usage: scripts/restore.sh TRUSTED_BACKUP_DIRECTORY' >&2; exit 1; }
case "$PGDATABASE" in *://*) echo 'PGDATABASE must be a database name, not a URL.' >&2; exit 1;; esac
(cd "$1" && sha256sum --check SHA256SUMS)
# Exclude only PostgreSQL/system schemas, including relations outside public.
count=$(psql -X -A -t -v ON_ERROR_STOP=1 -c "SELECT count(*) FROM pg_class c JOIN pg_namespace n ON n.oid=c.relnamespace WHERE n.nspname NOT LIKE 'pg_%' AND n.nspname <> 'information_schema' AND c.relkind IN ('r','p','v','m','S','f')")
[ "$count" = 0 ] || { echo 'Refusing to restore into a populated database.' >&2; exit 1; }
[ ! -L "$IMAGE_STORAGE_DIR" ] || { echo 'Destination storage must not be a symlink.' >&2; exit 1; }
mkdir -p "$IMAGE_STORAGE_DIR"
[ -z "$(find "$IMAGE_STORAGE_DIR" -mindepth 1 -maxdepth 1 -print -quit)" ] || { echo 'Destination storage must be empty.' >&2; exit 1; }
# Only use bundles created by backup.sh that you trust; do not extract untrusted archives.
tar -xzf "$1/images.tar.gz" --no-same-owner --no-same-permissions --keep-old-files -C "$IMAGE_STORAGE_DIR"
pg_restore --exit-on-error --single-transaction --no-owner --no-privileges --dbname="$PGDATABASE" "$1/archive.dump"
echo 'Restore complete. Apply pending migrations, check matching data/files, then provision accounts if needed.'
