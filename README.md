# The Archive

A private, locally runnable character catalog and artwork archive. M1 supports people, franchises, characters, tags, and extensible association types. M2 adds shared artwork stored on local disk, with metadata and character memberships in PostgreSQL. M3 protects the archive with provisioned accounts and PostgreSQL-backed sessions, and prepares a single-instance container deployment. Catalog operations, single-image upload, and editing use server-rendered HTML and ordinary forms without JavaScript. Optional bulk uploading uses a small browser script with no frontend build pipeline.

Product direction and milestone sequencing: [The Archive / The Librarian roadmap](docs/ROADMAP.md). Librarian Clerk M1 adds optional deterministic, read-only Discord retrieval: [setup and command guide](docs/DISCORD.md), [implementation brief](docs/LIBRARIAN_M1_PROMPT.md). Clerk uses SQL, not an AI model; later roadmap capabilities remain plans. Real Discord setup/registration/hosted acceptance are owner steps.

## Prerequisites and startup

Install stable Rust (validated with Rust 1.96.0), Docker with the Compose plugin, and PostgreSQL 17 client tools (`pg_dump`, `pg_restore`, `psql`) for recovery tests and backups. The database uses PostgreSQL 17; the application runs on the host.

```sh
cp .env.example .env
docker compose up -d --wait
# Hidden prompts ask for a password and confirmation. No accounts are seeded.
cargo run --locked -- account create owner-one
cargo run --locked -- account create owner-two
cargo run --locked
```

Open <http://127.0.0.1:3000> and sign in. Accounts are independent of catalog people; create people separately for associations and manual uploader attribution. Create franchises and people, then characters. Character pages let you add multiple person/type associations and tags. Create extra tags or association types from the navigation. Association keys are immutable; labels can be edited. Character filters combine with AND, and person/type filters match the same association row. Results are alphabetically ordered with stable tie breakers.

`IMAGE_STORAGE_DIR` defaults to `./var/images`, relative to the working directory. Set it to an absolute path if starting from different directories. Startup creates and canonicalizes it and probes file creation, syncing, publication, and removal; an unusable directory fails startup rather than choosing a fallback. Files use generated UUID names, and this directory is ignored by Git. Keep it writable by the application and do not let other users modify its contents. The storage filesystem must support hard links within that directory.

`APP_ENV` must explicitly be `development` or `production`; `.env.example` selects development. `DATABASE_URL` is required. `LISTEN_ADDR` defaults to `127.0.0.1:3000`; `APP_ORIGIN` defaults to the HTTP origin of that listener. Set both if using another address. `RUST_LOG` controls logs. `.env` is loaded without overriding exported environment variables. Startup connects to PostgreSQL and applies all pending ordered migrations before listening; connection or migration failure exits with an error. Migrations seed only the five standard association types. Compilation does not need PostgreSQL, and migration edits trigger recompilation.

Stop the application with Ctrl-C (or SIGTERM). `docker compose stop` stops PostgreSQL without deleting its named volume. Restart with `docker compose up -d --wait` and `cargo run --locked`; catalog data persists. `docker compose down` also preserves the volume. **Do not use `down -v` unless you intend to erase all data.**

For a complete manual M1–M3 walkthrough, see [testing.md](testing.md).

## Optional Discord retrieval

[The Librarian setup guide](docs/DISCORD.md) covers the four read-only guild commands: `/character`, `/images`, `/random-image`, and `/characters`. The integration defaults off. Enable it with the application ID/public key, one guild ID, and both allowed Discord user IDs in `.env.example`; the owner handles portal installation/endpoint validation and deployment. Website accounts and catalog people remain separate from Discord access principals.

```sh
cargo run --locked -- discord commands print
# Only after owner setup, with DISCORD_BOT_TOKEN read privately as documented:
cargo run --locked -- discord commands register
```

Printing is offline; registration does not initialize DB/storage and only upserts those four guild commands. No token is needed by the running server. Removing a website account does not revoke Discord access; edit the allowlist/redeploy or disable the integration. Ephemeral attachments copy artwork to Discord; recipients can retain/forward copies. Restarts can interrupt deferred retrieval; rerun the command. No domain migration or model service is added. Protocol handlers live in `src/discord`; reusable bounded exact queries live in `src/app/retrieval`.

## Tests and checks

Integration tests require a running real PostgreSQL server. SQLx creates separate uniquely named test databases, applies migrations to each, and cleans up successful tests. Tests never truncate or clear the catalog database. The test login needs `LOGIN` and `CREATEDB`, and access to the `postgres` maintenance database. The local Compose user is a superuser suitable for these local tests; do not reuse these credentials outside local development. Failed SQLx test databases may be retained for inspection.

```sh
docker compose up -d --wait
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
DATABASE_URL=postgres://archive:archive_local@127.0.0.1:5432/postgres cargo test
```

Tests provision real accounts and obtain cookies through the login route; authentication is never disabled in tests. Recovery tests invoke the installed PostgreSQL client tools (set `PG_BIN=/path/to/bin` if they are outside PATH). Tests cover accounts, password resets/disabling, session digests/expiry/revocation/restart persistence, cookies, protected routes, safe returns, rate limiting, maintenance mode, configuration, CLI administration, and a real M2 dump/restore followed by M3 migration/bootstrap. They also cover M1 behavior plus image constraints, all three formats, complete decoding and rejection of corrupt/animated inputs, injectable size/dimension/pixel/allocation policies, filename/path handling, metadata validation and escaping, uncategorized/shared images, duplicate races, transaction failure cleanup, deletion and missing files, gallery pagination, orphan maintenance, and a PostgreSQL-backed multipart HTTP workflow. Image tests generate tiny fixtures locally and use isolated temporary directories, never the configured development image archive. Clerk tests additionally use generated Ed25519 keys and a fake loopback Discord API to cover signatures/authorization, exact queries, bounded attachments, admission/replay/delivery errors, and native signed-PING/maintenance/restart smoke. No test contacts real Discord or needs a real token. If PostgreSQL is unavailable, integration tests fail rather than silently skip.

The optional browser regression harness uses Python 3 and an installed Chromium- or Firefox-compatible browser, serves only temporary test/code files on loopback, and fakes upload responses (no Archive/Discord access):

```sh
BROWSER=/path/to/chromium python3 scripts/test-bulk-browser.py
# Alternative (the implementation environment used a Firefox-compatible browser):
BROWSER=/path/to/firefox python3 scripts/test-bulk-browser.py --firefox
```

## Boundaries and request protection

Private catalog pages, galleries, originals, and mutations require a provisioned login. There is no public registration or password-reset email. Browser-public routes are `/login`, `/static/app.css`, `/static/bulk-upload.js` (code only), and `/healthz`. Optional POST `/discord/interactions` uses raw Ed25519 verification plus independent app/guild/user allowlists, without browser cookies; it defaults to 404. Unauthenticated page reads redirect to login; original-image reads and mutations return 401. Sign out uses POST. Authenticated HTML and authentication responses use `private, no-store`; images retain private caching and vary by Cookie. Health is a process-liveness check returning only `ok`: startup connects and migrates before listening, but health does not continually probe PostgreSQL.

The default listener and Compose database port bind to loopback. Production requires HTTPS `APP_ORIGIN`, an explicit storage directory, and a PostgreSQL URL explicitly requiring TLS. Cookie security is derived from `APP_ENV`, never forwarded headers: HttpOnly, SameSite=Lax, Path=/, and Secure in production, with no Domain. Follow [deployment.md](deployment.md) before exposing the application. This remains a small, single-instance private application rather than a multi-tenant service.

Unsafe requests must have an `Origin` header matching `APP_ORIGIN`. If Origin is absent, a Referer URL with the matching origin is accepted. A missing, malformed, `null`, or mismatched header is rejected with 403; an invalid Origin never falls back to Referer. Ordinary browsers send these headers for forms. Privacy configurations that strip both will prevent form submissions. Scripts must send a matching header **and a valid session cookie** for protected operations. Login is also subject to this check. Post-login return paths are limited to validated local catalog/image paths; external URLs and encoded path bypasses are rejected.

GET reads and POST mutates. Ordinary form successes redirect with 303; explicitly negotiated bulk-upload JSON uses 201/200. Validation returns 422, missing records 404, duplicate entities and restricted deletions 409, and unexpected failures 500 with details confined to server logs. Character and tag deletion cascades memberships; people, franchises, and association types in use cannot be deleted. A person attributed as an image uploader is also protected from deletion. Deleting a character retains its images and removes only its image memberships. Deleting an image retains its characters. Deletion requires an explicit confirmation page.

## Structure

One Rust package contains library and binary targets. `config` and `main` handle startup; `app` contains ordinary-input application functions and validation; `models` and `error` define data and typed failures. `web` handles forms, routes, and Askama rendering. SQLx runtime queries use explicit columns and bound values; dynamic table names come only from the internal catalog kind enum. PostgreSQL owns uniqueness, foreign keys, lengths, timestamps, and deletion rules. `templates`, `static`, and `migrations` contain HTML, CSS, and schema respectively. Dedicated image models live in `models/images`, application operations and validation in `app/images`, concrete local file operations in `storage`, and image handlers/templates in `web/images` and `templates/images`. Authentication application functions live in `app/auth`, HTTP/cookie/session protection in `web/auth`, and account CLI parsing/password input in `cli`. There is no parallel JSON API, frontend build, generic infrastructure framework, or cloud-storage integration.


## Using the image archive

1. Create people and characters such as Link and Venti in the catalog.
2. Follow **Images → Upload artwork**. Select one image, a person for uploader attribution, and zero or more character checkboxes. Uploader attribution is manually chosen, not verified identity; it can be corrected later. Artist and source URL are optional. Source URLs are recorded only, never fetched.
3. Open an image's detail page or a character's **Browse artwork** link. Shared artwork appears in each attached character's gallery. The gallery shows 24 images per page, newest first, with an ID tie breaker, and retains the character filter between pages.
4. Use **Edit metadata and characters** to correct uploader, artist, source, and memberships. Image bytes and the sanitized original filename stay unchanged.
5. Uploading the exact same bytes redirects to the existing image with an **Already archived** notice. Existing metadata and memberships stay unchanged; use Edit to add further characters. Different encodings of the same artwork are distinct in M2. Valid upload metadata/selections are required even when the bytes are duplicates.
6. **Delete** opens a confirmation page. Removing an image deletes its database metadata and memberships, then attempts to remove its file. Missing files do not block deletion. If file removal fails, the gallery reports that the record was deleted but cleanup is needed.

Uploads accept static JPEG, PNG, and WebP, detected from bytes rather than filenames or supplied MIME types. Original bytes are stored and SHA-256 is computed over those exact bytes. The database stores SHA-256 as a 32-byte `BYTEA`, with length and uniqueness constraints. SVG, GIF, other formats, APNG, and animated WebP are rejected. Original filename path components (both slash forms) and control characters are discarded; the display filename is at most 255 Unicode characters, with `upload` as a fallback. It is never used as a storage path or response header.

Limits are **20 MiB per file**, **12,000 pixels per axis**, and **40 million decoded pixels**. Multipart requests are capped at **20 MiB + 64 KiB**, including chunked bodies; metadata fields are also individually bounded at 16 KiB, collectively at 64 KiB, and the upload accepts at most 1,024 parts. Exactly one file is allowed. Invalid upload forms retain submitted text and valid selections, but browsers cannot repopulate file inputs: select the file again after an error.

Validation inspects dimensions before allocating a full output buffer and actually decodes the image. PNG validation also consumes the final chunks and end marker; JPEG uses strict decoder options and checks its end marker; WebP validates container lengths, animation markers, and coded-frame dimensions as well as the declared canvas. Decoding and hashing run on blocking threads, with at most two concurrent decodes.

The output-buffer allocation has a strict **512 MiB** ceiling. PNG and WebP decoder budgets are set to 512 MiB as well, but **internal decoder allocation limits are best-effort**, not a guaranteed process/RSS ceiling; JPEG has no independent internal allocation-budget API. Strict encoded-size, dimension, pixel, and output bounds constrain inputs, but M2 does not provide OS-level memory isolation for decoder internals. See the upstream [image allocation-limit documentation](https://docs.rs/image/0.25.10/image/struct.Limits.html) and [WebP decoder memory-limit documentation](https://docs.rs/image-webp/0.2.4/image_webp/struct.WebPDecoder.html#method.set_memory_limit). This is a remaining limit of the local in-process decoder approach, not a claim of a hard 512 MiB total-memory guarantee.

Images are streamed through `/images/{id}/content`, with detected Content-Type and `X-Content-Type-Options: nosniff`. Storage keys are constrained generated relative filenames; arbitrary paths, symlinks, and directories are not served. Metadata remains visible if a file is missing; the content route returns a controlled 500 and logs the issue. Galleries use lazy loading and serve originals in M2; there is no thumbnail pipeline or range-request support.

## Bulk artwork upload

Images → **Bulk upload artwork** (`/images/bulk`) accepts up to 100 selected files. Choose one manual uploader attribution, optional shared artist/source, and any characters once. **Every newly archived image receives all selected characters.** For mixed character collections or different artists/sources, use separate batches or edit each image afterward; no provenance is inferred from filenames. Zero characters is allowed.

Review filenames/sizes and remove unwanted files before starting. The browser snapshots shared values for each run and disables editing while it sends **one file per request, sequentially**, through the ordinary authenticated/same-origin `/images` upload. Existing 20 MiB file / 20 MiB + 64 KiB request / decoder limits are unchanged. There is no batch-sized server buffer, schema migration, persistent batch job, new environment variable, or infrastructure requirement. Multiple browser pages still share the existing two-decoder server limit; this feature is not a deployment-tier memory measurement.

Each file commits independently. Outcomes are pending, uploading, uploaded, duplicate, failed, or unconfirmed, with a completed/total summary and record/editor links. Exact duplicates within a selection or already in the archive are successful **duplicate** outcomes: their attribution, artist/source, and memberships remain unchanged. Use their Edit link to correct assignments. Ordinary validation/413 failures do not stop subsequent files; earlier successful files stay saved.

**Stop** stops scheduling and lets the current request settle. Session expiry/access rejection, maintenance, unexpected responses, and network/server failures pause scheduling. Keep the page open; sign in again using its new-tab link if needed, then explicitly resume pending files or retry failed/unconfirmed files. Retry skips uploaded/duplicate files and uses the shared values currently shown. No network retries occur automatically. A response can be lost after a commit, so **unconfirmed does not mean unsaved**; retrying identical bytes resolves safely through duplicate detection. Requests time out after two minutes, also yielding an unconfirmed outcome.

File objects/results remain only in the open page. Reloading/navigating away loses the selection and can interrupt an upload; completed records/files persist. There is no background or cross-reload resume. Without JavaScript, follow the page's ordinary single-image upload link; single upload/edit flows are unchanged. See [the browser/manual checklist](testing.md#bulk-upload-regression-checklist).

The upload route returns bounded JSON only with explicit `Accept: application/json`: 201 `{"status":"uploaded","id":"…"}`, 200 `{"status":"duplicate","id":"…"}`, or the existing error status with `{"status":"failed","message":"…"}`. IDs are decimal strings to preserve PostgreSQL bigint precision. Normal HTML clients retain 303 redirects/rendered validation errors. Middleware can still return plain-text 401/403/413/503; the browser checks status, redirects, content type, and confirmation shape rather than treating a login page as success.

## File/database consistency and maintenance

Filesystem operations and PostgreSQL transactions are not atomic together. The upload strategy is:

- Validate the file and all metadata/referenced records before writing permanent storage.
- Write and sync a generated temporary file, publish the generated final filename using a hard link that cannot overwrite another file, and remove the temporary name.
- Insert image metadata and all character memberships in one transaction. The unique SHA-256 constraint settles concurrent duplicate uploads; only the winner's metadata/memberships remain.
- Remove the newly generated file on ordinary database or duplicate-race failures. Cleanup never deletes the existing duplicate's key. If PostgreSQL explicitly rejects a commit, clean up the new file. If its acknowledgement is lost through a network/protocol failure, retain the file and log the ambiguous outcome; a separate connection seeing no row is not proof that an in-flight commit cannot finish later. Inspect metadata after recovery, and let offline orphan maintenance reconcile any unreferenced file.

Crashes, cancellation, disk errors, or failed cleanup can leave temporary files or unreferenced final files. This is **not crash-proof atomicity**. File contents are synced, but directory entries are not separately fsynced, so a power failure can also leave missing files; restore those from backup. The orphan command removes unreferenced files, not metadata for missing files.

Run maintenance from the same working directory/environment and with the same `DATABASE_URL` and `IMAGE_STORAGE_DIR` as the application:

```sh
# Safe default: print candidates; do not remove them.
cargo run --locked -- cleanup-orphans

# Stop the application/uploads first, inspect the dry run, then explicitly apply.
cargo run --locked -- cleanup-orphans --apply
```

Maintenance compares files against database storage keys. It considers only regular files with generated `<32 lowercase hex digits>.jpg/png/webp` names or `.upload-<32 hex digits>.tmp` names. It never recurses, never follows symlinks, and ignores unrelated files. A fixed **24-hour grace period** excludes recent and future-dated temporary/orphan files. **Only run destructive cleanup while uploads are stopped**: grace is a safeguard, not coordination with live transactions. Use the correct database/storage pair; another database cannot identify your archive's referenced files.

## Accounts and sessions

```sh
cargo run --locked -- migrate
cargo run --locked -- account create owner-one
cargo run --locked -- account list
cargo run --locked -- account reset-password owner-one
cargo run --locked -- account disable owner-one
cargo run --locked -- sessions cleanup
```

Database-only commands require only `DATABASE_URL`, load `.env` without replacing exports, apply pending migrations, and never initialize image storage. Creation/reset uses hidden, confirmed prompts. For automation, `account create USER --password-stdin` and `account reset-password USER --password-stdin` read one UTF-8 password line (optional LF/CRLF) followed by EOF, with a bounded read; pipe directly from a password manager or use a private temporary input file. Never put passwords in arguments, exported variables, or shared shell history. A reset leaves a disabled account disabled.

Usernames are trimmed, lowercase ASCII, 3–64 characters: letters/digits plus dot, underscore, hyphen, starting with a letter/digit. Passwords are 12–1,024 UTF-8 **bytes**, without NUL; spaces are preserved and there are no composition rules. Interactive/stdin input is one line. Argon2id v19 uses 19,456 KiB, two iterations, one lane, a random 16-byte salt and 32-byte output. Password work runs on blocking threads with at most two concurrent operations.

Each login creates a new OS-random 32-byte opaque token encoded as 64 lowercase hex characters. Only its SHA-256 digest is persisted. Sessions expire absolutely after seven days and survive restarts. Logout revokes the current session; password reset/disable revokes all account sessions transactionally. Expired or disabled sessions cannot authenticate even before cleanup. Successful logins delete up to 100 expired rows; `sessions cleanup` deletes another bounded batch per invocation. No signing secret is needed.

A global in-process rolling limiter admits 20 login submissions per minute, shared across users. Its fixed-capacity queue does not use caller-supplied IP/forwarding headers. It resets on restart, and one user can temporarily exhaust it for everyone; this is intentional for a two-person, single-instance archive. It is not distributed abuse protection.

## Backups, recovery, and deployment

Back up PostgreSQL **and** the image directory together while all application writers are stopped. `MAINTENANCE_MODE=true` returns 503 for browser routes except health/CSS, including login/logout; changing it requires a restart. Signed Discord PING still responds, while authorized commands return an ephemeral maintenance notice without retrieving data. Wait for existing retrieval jobs to drain (up to 60 seconds) before file maintenance. Do not run account/migration/cleanup commands concurrently with a coordinated snapshot. The database must remain running. Use [scripts/backup.sh](scripts/backup.sh) and [scripts/restore.sh](scripts/restore.sh); restoration refuses a populated database or nonempty image destination. Dumps include sensitive private data, password hashes and session digests. Keep them encrypted/private and outside Git and the live archive.

[deployment.md](deployment.md) contains exact local and Railway account, backup, isolated restore, and M2 import instructions, provider steps, volume ownership handling, and outstanding verification. A restored M2 dump retains its applied migration history; migrate only **after** restoring into the empty destination. Code rollback never reverses database migrations.

The multi-stage [Dockerfile](Dockerfile) builds locked release code, includes TLS certificates and PostgreSQL recovery tools, and defaults to UID 10001. Railway volume initialization can start the entrypoint as root using `RAILWAY_RUN_UID=0`; it fixes only `/data/images`, then drops privileges before launching the application. No deployment was performed as part of M3. Docker/Compose/container and provider-specific behavior must be checked by the owner where those tools/services are available.

People remain domain subjects, not accounts. There are no cloud storage services, source downloads, image edits/crops, perceptual hashes, image tags, representative portraits, background queues, or frontend build pipeline. Galleries still serve originals; thumbnail work and deployment-tier decoder memory measurement remain open in [technical-debt.md](technical-debt.md).
