# Private deployment and recovery

M3 implemented 2026-10-04. This is an operator runbook, not a claim that the application has been deployed. Target: **one Railway instance**, a **direct Neon PostgreSQL connection**, and a **Railway persistent volume at `/data`**. Accounts are independent of catalog people. There is no public registration, cloud object storage, or authentication bypass for tests.

## Implemented and verified locally

- Argon2id accounts; hidden-prompt/stdin CLI bootstrap/reset/list/disable; revocable, seven-day PostgreSQL sessions storing token digests only.
- Authentication on catalog, galleries, image originals and mutations; same-origin checks including login/logout; production Secure cookies; private cache policies; bounded global login limiter.
- Environment-only runtime configuration, database-only migration/admin commands, startup migrations before listening, graceful SIGTERM/Ctrl-C shutdown, non-sensitive `/healthz` liveness endpoint.
- Locked release Dockerfile, runtime certificates/PostgreSQL client tools, non-root application user, targeted Railway volume initialization, build exclusions for secrets and personal data.
- Coordinated dump/file backup and isolated restore scripts with overwrite guards. Real PostgreSQL tests restore an M2 fixture, verify catalog/image metadata/memberships and exact bytes, apply M3 migration, and provision a login afterward.

Docker is unavailable in the implementation environment. **Container build/runtime, Railway UID/volume/hard-link behavior, HTTPS proxy/domain routing, actual Neon TLS connectivity, provider upload timeouts/limits, and deployment-tier memory have not been verified.** Native Rust/PostgreSQL checks cannot substitute for these checks. No provider accounts, credentials, or deployments were created.

## Values and provider actions the owner must supply

1. Create Railway/Neon accounts, enable MFA where available, review budget/memory/backup retention, and authorize Railway to access the GitHub repository. Choose nearby regions. Use one service instance; do not enable replicas.
2. Create a Neon PostgreSQL 17 database and obtain its **direct**, non-pooler hostname/URL. Keep credentials in the provider Variables UI/password manager, not Git. The application pool is capped at ten connections. See [Neon connections](https://neon.com/docs/connect/connect-from-any-app).
3. Create the Railway application from GitHub using this Dockerfile. Attach a persistent volume at `/data`. Configure `/healthz` as the health-check path (120-second startup allowance is reasonable) and target port **3000**. Health means process liveness, not a continuously tested database. Migrations/connection/storage must succeed before the listener exists.
4. Generate the Railway HTTPS domain (or configure a custom domain), enter its exact origin below, and keep the service in maintenance mode until bootstrap/import is finished. `APP_ORIGIN` trusts configuration only; forwarded host/protocol headers cannot change it. See [Railway public networking](https://docs.railway.com/networking/public-networking).

| Variable | Owner-supplied production setting |
| --- | --- |
| `APP_ENV` | `production` (required; no development fallback) |
| `DATABASE_URL` | Direct Neon URL, preferably `postgresql://<user>:<encoded-password>@<direct-host>/<database>?sslmode=verify-full` |
| `LISTEN_ADDR` | `0.0.0.0:3000`; literal numeric port, no `$PORT` expansion |
| `APP_ORIGIN` | `https://<final-domain>`; scheme/host/optional port only |
| `IMAGE_STORAGE_DIR` | `/data/images` |
| `RUST_LOG` | `the_archive=info,tower_http=info`; avoid verbose third-party logs |
| `RAILWAY_RUN_UID` | `0` for the permission-initializing entrypoint described below |
| `MAINTENANCE_MODE` | `true` during bootstrap/import/backups; `false` for use |

No application signing secret is needed. All browser cookies are HttpOnly, SameSite=Lax, Path=/, with Secure enforced in production. Missing/insecure/malformed production origins and database URLs without required TLS fail configuration. Production requires an explicit storage path. `.env` is optional and never replaces explicit variables; `.dockerignore` excludes it.

SQLx is compiled with `tls-rustls-ring` and WebPKI trust roots; runtime CA certificates are also installed for PostgreSQL client tools. The supplied connection string reaches SQLx without rewriting/removing TLS parameters. Prefer `sslmode=verify-full` (hostname/certificate verification); `require`/`verify-ca` are accepted explicitly but offer weaker verification. **SQLx 0.8 does not implement SCRAM channel binding**: URLs containing `channel_binding` are rejected clearly rather than silently ignoring the requested policy. Obtain a Rust-compatible Neon connection string without that parameter, retaining TLS requirements; if your policy requires channel binding, this version is not sufficient. Do not simply remove TLS parameters. See [SQLx connection/TLS options](https://docs.rs/sqlx/0.8.6/sqlx/postgres/struct.PgConnectOptions.html) and [its SASL implementation](https://github.com/launchbadge/sqlx/blob/v0.8.6/sqlx-postgres/src/connection/sasl.rs). Actual Neon handshake verification remains an owner step.

## Container and volume permissions

```sh
# Local build, when Docker is installed:
docker build --pull -t the-archive:m3 .
# Prepare an empty persistent host directory owned by the runtime UID, without chmod 777:
sudo install -d -m 700 -o 10001 -g 10001 /srv/the-archive/images
# Use a private environment file containing production values and a reachable database.
docker run --rm --name the-archive -p 127.0.0.1:3000:3000 \
  --env-file /private/path/archive-production.env \
  --mount type=bind,src=/srv/the-archive/images,dst=/data/images \
  the-archive:m3
```

The default container USER is 10001:10001. The owner must supply reachable PostgreSQL and HTTPS reverse-proxy origin for a production run; localhost inside the container is not the host database. Build uses Rust 1.96.0 and Cargo.lock, Debian trixie runtime, certificates, `gosu`, and PostgreSQL 17 client tools. Personal data is never copied into the image. Dependencies are locked; base-image/OS package security updates can change a rebuild's exact bytes.

Railway documents root-owned volume mounts and a `RAILWAY_RUN_UID=0` override. Our entrypoint uses that override **only to initialize `/data/images`**, gives UID 10001 group-only traversal of the mount root (`root:10001`, 0710), sets the image directory to 0700, adjusts ownership of its direct regular files to 10001, and `exec gosu 10001:10001 /app/the-archive …`. It does not chown the entire volume, follow symlinks, or make files world-writable. The server runs as the non-root archive user. Root initialization refuses a different storage path; manually prepare permissions if choosing another mount layout. Admin/migration commands skip image permission initialization entirely. See [Railway volumes](https://docs.railway.com/volumes).

Before relying on this setup, use Railway SSH to verify `ps` shows the server UID as 10001, `stat -c '%u:%g %a' /data /data/images` reports `0:10001 710` then `10001:10001 700`, and startup's create/sync/hard-link/remove probe succeeds. After import, restart the entrypoint to fix imported file ownership. Verify upload, streaming, and restart persistence. Railway permissions and hard links cannot be confirmed without the real mounted volume.

Volumes exist **at runtime**, not in build/pre-deploy phases. Do not use storage-dependent commands in pre-deploy. Startup applies ordered migrations for this single-instance service and exits if they fail; database-only `migrate` is available separately. Code rollback does not roll back schema changes. Back up before deploying a migration.

## Bootstrap and account administration

Local setup:

```sh
cp .env.example .env
docker compose up -d --wait
cargo run --locked -- account create owner-one
cargo run --locked -- account create owner-two
cargo run --locked
```

Prompts hide passwords and ask for confirmation. Use unique password-manager passwords of 12–1,024 UTF-8 bytes; there are no complexity rules. Usernames normalize to lowercase ASCII (3–64 characters). No accounts/passwords are seeded. A catalog person is still required separately for manual uploader attribution.

Install and authenticate the Railway CLI yourself, then link the correct project/environment/service. Confirm this selection before every administrative/transfer operation. [Railway SSH](https://docs.railway.com/cli/ssh) executes inside the running container with its environment and volume, unlike `railway run`, which runs locally.

```sh
railway ssh
# In the remote interactive shell; this wrapper drops to the application UID:
/app/scripts/container-entrypoint.sh migrate
/app/scripts/container-entrypoint.sh account create owner-one
/app/scripts/container-entrypoint.sh account create owner-two
/app/scripts/container-entrypoint.sh account list
/app/scripts/container-entrypoint.sh account reset-password owner-one
/app/scripts/container-entrypoint.sh account disable owner-one
/app/scripts/container-entrypoint.sh sessions cleanup
```

Interactive SSH supplies a TTY for hidden prompts. If your client lacks a usable TTY, use `account create USER --password-stdin` / `reset-password USER --password-stdin` with one password line then EOF, piped directly from a password manager or a mode-0600 private input file. Do not put a password in an argument, provider variable, command history, or example. Database-only commands need `DATABASE_URL`, never a readable/writable image directory. Reset/disable revokes all account sessions; reset does not re-enable a disabled account. Expired-session cleanup removes at most 100 per invocation, also run after successful logins.

Set `MAINTENANCE_MODE=false` and redeploy only after accounts/import checks. Verify logged-out requests cannot read data or originals, then sign in from both devices. The global 20-attempt/minute limiter resets on restart and can briefly block both users; it is not a distributed rate limiter.

## Coordinated backup (local or Railway)

**Stop all application writers**, including other app copies and admin/migration commands. Locally stop the Rust process; leave PostgreSQL up. On Railway set `MAINTENANCE_MODE=true`, redeploy, wait for the new instance, and verify both a catalog GET and login POST return 503. Health remains 200. Ensure old deployments have stopped. The mode blocks login/logout and session cleanup too; do not issue admin commands during the snapshot.

The scripts use libpq `PGHOST`, `PGPORT`, `PGUSER`, `PGDATABASE`, `PGSSLMODE` and `PGPASSFILE`; secrets need not appear in arguments. `PGDATABASE` must be a database name, never a URL. For Neon use its direct hostname and `PGSSLMODE=verify-full`. Create the password file interactively/private editor with mode 0600, containing `host:port:database:user:password` (escape `:` and `\` as libpq requires). Do not commit it. See [PostgreSQL password-file rules](https://www.postgresql.org/docs/17/libpq-pgpass.html). Avoid printing credentials or full `DATABASE_URL` in shared logs.

Example local Compose database (enter the local password from `.env.example` in a private password file):

```sh
export PGHOST=127.0.0.1 PGPORT=5432 PGUSER=archive PGDATABASE=archive PGSSLMODE=disable
export PGPASSFILE=/private/path/archive.pgpass
chmod 600 "$PGPASSFILE"
export IMAGE_STORAGE_DIR="$PWD/var/images"
export ARCHIVE_WRITES_STOPPED=yes
scripts/backup.sh "$PWD/backups/archive-2026-10-04"
```

For Railway, run the same script in `railway ssh`, substituting Neon PG variables and `IMAGE_STORAGE_DIR=/data/images`:

```sh
# Set PGHOST/PGUSER/PGDATABASE/PGSSLMODE/PGPASSFILE for the same direct database.
export PGPORT=5432 PGSSLMODE=verify-full IMAGE_STORAGE_DIR=/data/images
export ARCHIVE_WRITES_STOPPED=yes
mkdir -p /data/backups
/app/scripts/backup.sh /data/backups/archive-2026-10-04
```

Each **new** backup directory contains `archive.dump`, `images.tar.gz`, and `SHA256SUMS`. The dump includes migration history, identity counters, accounts/hashes, and session digests; files retain generated storage keys. The script refuses an existing bundle directory. It does not stop writers for you; `ARCHIVE_WRITES_STOPPED=yes` records your explicit operational acknowledgement, not a lock. If the command fails, treat its partial bundle as invalid and use a new directory on retry. Store completed bundles privately/encrypted outside the live volume and confirm checksums after copying.

Download the three files with Railway's documented [service file commands](https://docs.railway.com/cli/service) (from your local terminal):

```sh
mkdir -m 700 ./backups/railway-2026-10-04
railway service files download /data/backups/archive-2026-10-04/archive.dump ./backups/railway-2026-10-04/archive.dump
railway service files download /data/backups/archive-2026-10-04/images.tar.gz ./backups/railway-2026-10-04/images.tar.gz
railway service files download /data/backups/archive-2026-10-04/SHA256SUMS ./backups/railway-2026-10-04/SHA256SUMS
(cd ./backups/railway-2026-10-04 && sha256sum --check SHA256SUMS)
```

Provider transfer commands are documented but were not executed here. Check installed CLI help and the linked service/environment before transferring. Alternatively use the SCP/SFTP mechanism in Railway's SSH documentation. Restore rehearsals should use private copies of the matching three-file bundle.

## Isolated restore and local rehearsal

Never restore over a populated or active production database. Use a newly created empty database and empty storage directory, with the app stopped. Use PostgreSQL 17 client tools (or a compatible version no older than the source server). The scripts reject populated databases, symlink/nonempty image destinations, checksum failures, and missing writer acknowledgement. They do not erase existing data. Only extract **trusted** bundles created by the backup script.

```sh
# Set PGHOST/PGPORT/PGUSER/PGPASSFILE first. Choose a NEW database name.
export PGDATABASE=archive_restore_20261004
createdb "$PGDATABASE"
export IMAGE_STORAGE_DIR="$PWD/var/restore-20261004/images"
export ARCHIVE_WRITES_STOPPED=yes
scripts/restore.sh "$PWD/backups/archive-2026-10-04"
# Only AFTER restore, use the matching connection URL (load it from private config).
# Set DATABASE_URL to that isolated database, APP_ENV=development,
# LISTEN_ADDR=127.0.0.1:3001, APP_ORIGIN=http://127.0.0.1:3001.
cargo run --locked -- migrate
# For an older M2 archive (no accounts), create logins now:
cargo run --locked -- account create owner-one
cargo run --locked -- account create owner-two
cargo run --locked
```

Do not run migrations against the empty destination before restoring: that creates tables and conflicts with the dump. M2's `_sqlx_migrations` history is restored first, then the M3 migration adds accounts/sessions; existing M2 migrations are unchanged. A current M3 backup already contains accounts; do not recreate duplicate usernames. For a security-sensitive recovery, reset each account password before exposing it, which also revokes restored sessions. Verify catalog counts/records, dimensions/artist/uploader, character memberships, stored key and SHA-256, and exact served bytes. The application probes storage on startup and preserves files across restarts.

Automated evidence: `tests/recovery.rs::coordinated_m2_backup_restore_and_bootstrap` creates an isolated M2 database and tiny PNG, runs these actual scripts with `pg_dump`/`pg_restore`, restores into another empty database/temp directory, verifies Link, uploader/artist, image membership, generated key/hash and exact file bytes, then applies M3 and creates/logs in an account. It verifies a second restore is refused. Run:

```sh
DATABASE_URL=postgres://archive:archive_local@127.0.0.1:5432/postgres cargo test --test recovery
```

This demonstrates ordinary coordinated restoration, **not power-loss/crash atomicity** or production-tier memory safety.

## Import an existing local M2 archive into Neon and the Railway volume

Avoid the bootstrap/migration ordering trap: a server pointed at the destination migrates it immediately. Therefore use **two databases** during initial import:

1. Stop local writers and create the coordinated three-file bundle above. Keep the source intact until verification.
2. Create an empty Neon **final destination** and a separate temporary **bootstrap database**. Start Railway with `DATABASE_URL` pointing to the bootstrap database, `MAINTENANCE_MODE=true`, configured final HTTPS origin, and the mounted empty `/data/images`. Startup migrations touch only bootstrap; the final destination stays empty. Do not let any app instance connect to/migrate the final destination yet.
3. Create `/data/import-m2` in the remote SSH shell (`mkdir -m 700 -p /data/import-m2`), then upload the bundle through the linked service's CLI. Use the documented upload command for each file:

   ```sh
   railway service files upload ./backups/archive-2026-10-04/archive.dump /data/import-m2/archive.dump
   railway service files upload ./backups/archive-2026-10-04/images.tar.gz /data/import-m2/images.tar.gz
   railway service files upload ./backups/archive-2026-10-04/SHA256SUMS /data/import-m2/SHA256SUMS
   ```

4. In `railway ssh`, set **libpq PG variables**/private `PGPASSFILE` to the **empty final destination**, not bootstrap. Set `IMAGE_STORAGE_DIR=/data/images` and `ARCHIVE_WRITES_STOPPED=yes`, then run `/app/scripts/restore.sh /data/import-m2`. The checksum/empty guards apply. Preserve all generated filenames; do not rename or re-encode images. Import never fetches source URLs.
5. Now update the Railway service `DATABASE_URL` to the final destination and redeploy, still in maintenance mode. Entrypoint fixes imported file ownership. Startup applies only pending M3 migration after the restored M2 history. Use the account commands in SSH to provision two logins, then inspect metadata/files through an isolated validation session or enable access only for your two accounts. A database-only `migrate` command after restore is also safe, with `DATABASE_URL` explicitly targeting final.
6. Set maintenance false, redeploy, verify login/shared Link+Venti artwork/gallery/edit/duplicate upload/content/deletion/logout, then restart and confirm data and sessions persist. Verify unauthorized original requests return 401. After a verified matching backup, retire the temporary bootstrap database through Neon yourself. Do not delete the original local archive or import bundle until recovery is proven.

The same bootstrap-database pattern supports restoration onto a new empty volume/database when runtime SSH is needed. For recovery onto an existing populated volume, use a new volume/service or move the old data aside through an explicit operator decision; this runbook does not automatically delete or overwrite it. Never swap the live service to an empty final DB before restore.

## Orphans, missing files, and consistency limits

Uploads validate before persistence, sync/publish generated files, and transactionally insert metadata/memberships. Duplicate races remove only their own new files. Database/file operations are not atomic: crashes, cancellation, lost commit acknowledgement, or power loss may leave an orphan or a missing file. Ambiguous commits intentionally retain files. File contents are synced; directory entries are not separately fsynced. Image deletion commits metadata removal before file removal; a cleanup failure is reported as an orphan issue, not a failed database deletion.

With **all uploads stopped** (maintenance mode and no other writers), first inspect:

```sh
/app/scripts/container-entrypoint.sh cleanup-orphans
# Only after inspection, while uploads remain stopped:
/app/scripts/container-entrypoint.sh cleanup-orphans --apply
```

Only generated regular final/temp files are considered, without following links/recursion, and only after a **24-hour grace period**. Recent/future-dated files and unrelated files are ignored. Use the correct database/storage pair. Orphan cleanup cannot restore missing image bytes or prove that an uncertain commit failed. For missing content, keep the record while recovering its exact generated file from the matching backup, or explicitly delete the stale record through the UI; missing files do not prevent record deletion. For an uncertain commit, first check the recovered database before considering offline cleanup.

TD-002 thumbnails and TD-003 deployment-tier memory measurement remain open. TD-004 now has ordinary backup/restore evidence, but crash/power-loss experiments remain unverified. Before hosted reliance, test these procedures with the real volume and database, check restore permissions/bytes, and maintain backups outside the provider volume. GitHub code deployment/rollback never replaces data backup or reverses migrations.
