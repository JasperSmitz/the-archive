# The Archive

A private, locally runnable character catalog and artwork archive. M1 supports people, franchises, characters, tags, and extensible association types. M2 adds shared artwork stored on local disk, with metadata and character memberships in PostgreSQL. All operations use server-rendered HTML and ordinary forms; JavaScript is unnecessary.

## Prerequisites and startup

Install stable Rust (validated with Rust 1.96.0), Docker with the Compose plugin, and optionally `psql` for database inspection. The database uses PostgreSQL 17; the application runs on the host.

```sh
cp .env.example .env
docker compose up -d --wait
cargo run --locked
```

Open <http://127.0.0.1:3000>. Create franchises and people, then characters. Character pages let you add multiple person/type associations and tags. Create extra tags or association types from the navigation. Association keys are immutable; labels can be edited. Character filters combine with AND, and person/type filters match the same association row. Results are alphabetically ordered with stable tie breakers.

`IMAGE_STORAGE_DIR` defaults to `./var/images`, relative to the working directory. Set it to an absolute path if starting from different directories. Startup creates and canonicalizes it and probes file creation, syncing, publication, and removal; an unusable directory fails startup rather than choosing a fallback. Files use generated UUID names, and this directory is ignored by Git. Keep it writable by the application and do not let other users modify its contents. The storage filesystem must support hard links within that directory.

`DATABASE_URL` is required. `LISTEN_ADDR` defaults to `127.0.0.1:3000`; `APP_ORIGIN` defaults to the HTTP origin of that listener. Set both if using another address. `RUST_LOG` controls logs. `.env` is loaded without overriding exported environment variables. Startup connects to PostgreSQL and applies all pending ordered migrations before listening; connection or migration failure exits with an error. Migrations seed only the five standard association types. Compilation does not need PostgreSQL, and migration edits trigger recompilation.

Stop the application with Ctrl-C (or SIGTERM). `docker compose stop` stops PostgreSQL without deleting its named volume. Restart with `docker compose up -d --wait` and `cargo run --locked`; catalog data persists. `docker compose down` also preserves the volume. **Do not use `down -v` unless you intend to erase all data.**

## Tests and checks

Integration tests require a running real PostgreSQL server. SQLx creates separate uniquely named test databases, applies migrations to each, and cleans up successful tests. Tests never truncate or clear the catalog database. The test login needs `LOGIN` and `CREATEDB`, and access to the `postgres` maintenance database. The local Compose user is a superuser suitable for these local tests; do not reuse these credentials outside local development. Failed SQLx test databases may be retained for inspection.

```sh
docker compose up -d --wait
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
DATABASE_URL=postgres://archive:archive_local@127.0.0.1:5432/postgres cargo test
```

Tests cover M1 behavior plus image constraints, all three formats, complete decoding and rejection of corrupt/animated inputs, injectable size/dimension/pixel/allocation policies, filename/path handling, metadata validation and escaping, uncategorized/shared images, duplicate races, transaction failure cleanup, deletion and missing files, gallery pagination, orphan maintenance, and a PostgreSQL-backed multipart HTTP workflow. Image tests generate tiny fixtures locally and use isolated temporary directories, never the configured development image archive. If PostgreSQL is unavailable, integration tests fail rather than silently skip.

## Boundaries and request protection

This is a local-only application with **no authentication**. Anyone who can access the listener can read and change its data. Both the default application listener and Compose's database port bind to loopback. Do not expose either to a public network.

Unsafe requests must have an `Origin` header matching `APP_ORIGIN`. If Origin is absent, a Referer URL with the matching origin is accepted. A missing, malformed, `null`, or mismatched header is rejected with 403; an invalid Origin never falls back to Referer. Ordinary browsers send these headers for forms. Privacy configurations that strip both will prevent form submissions. Scripts must explicitly send a matching header, for example:

```sh
curl -i -H 'Origin: http://127.0.0.1:3000' \
  -d 'name=Zelda' http://127.0.0.1:3000/franchises
```

GET reads and POST mutates. Successful mutations redirect with 303. Validation returns 422, missing records 404, duplicate entities and restricted deletions 409, and unexpected failures 500 with details confined to server logs. Character and tag deletion cascades memberships; people, franchises, and association types in use cannot be deleted. A person attributed as an image uploader is also protected from deletion. Deleting a character retains its images and removes only its image memberships. Deleting an image retains its characters. Deletion requires an explicit confirmation page.

## Structure

One Rust package contains library and binary targets. `config` and `main` handle startup; `app` contains ordinary-input application functions and validation; `models` and `error` define data and typed failures. `web` handles forms, routes, and Askama rendering. SQLx runtime queries use explicit columns and bound values; dynamic table names come only from the internal catalog kind enum. PostgreSQL owns uniqueness, foreign keys, lengths, timestamps, and deletion rules. `templates`, `static`, and `migrations` contain HTML, CSS, and schema respectively. Dedicated image models live in `models/images`, application operations and validation in `app/images`, concrete local file operations in `storage`, and image handlers/templates in `web/images` and `templates/images`. There is no parallel JSON API, frontend build, authentication, or cloud infrastructure.


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

## Backups and recovery

Back up **PostgreSQL and the image directory together**, with the application stopped so they represent the same archive state. For the default local setup:

```sh
# Stop the Rust application with Ctrl-C. Leave PostgreSQL running.
mkdir -p backups
docker compose exec -T db pg_dump -U archive -d archive -Fc > backups/archive.dump
tar -czf backups/images.tar.gz -C var images
```

For a custom storage path, archive that configured directory instead. To restore, stop the application, restore the matching database dump with `pg_restore`, and restore its matching image directory before starting again. Keep backups outside the storage directory, and do not commit personal backups. A database-only backup cannot recover image bytes; a file-only backup cannot recover metadata and memberships.

M2 remains local-only and unauthenticated. People are domain subjects, not accounts. There are no cloud services, source downloads, image edits/crops, perceptual hashes, image tags, representative portraits, background queues, or frontend build pipeline.
