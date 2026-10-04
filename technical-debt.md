# Technical debt and review follow-ups

Updated 2026-10-04 during M3 implementation. Track remaining verification and concrete follow-ups here; deployment/account operation instructions belong in [deployment.md](deployment.md). M1/M2 migrations remain unchanged.

## Open

### TD-002 — Generate gallery thumbnails

Priority: before routine remote gallery use. Source: M2 review.

Gallery cards serve original files. Lazy loading helps, but 24 large originals can still require substantial bandwidth and browser decoding.

- Location: `templates/images/gallery.html`, `src/web/images.rs`, `src/app/images/`, `src/storage.rs`.
- Add bounded static thumbnails as derived assets, with the authenticated delivery added in M3. Preserve original bytes and their SHA-256. Start with synchronous generation or an explicit local backfill, without introducing a queue.
- Done when gallery requests use smaller assets, existing archives can be backfilled, originals remain available on detail pages, and deletion/maintenance handle derived files correctly.

### TD-003 — Verify decoder memory use on the deployment tier

Priority: deployment readiness. Source: documented M2 limitation.

Encoded-size/dimension/pixel/output limits exist, but internal decoder budgets do not guarantee a total process-memory ceiling. Two concurrent decodes plus collected uploads can exceed a small instance's memory.

- Location: `src/app/images/validation.rs`, decode semaphore in `src/app/images/mod.rs`, README upload limits.
- Measure representative large supported images on the chosen memory tier. Lower concurrency or limits if necessary; consider bounding admission before retaining many collected uploads if measurements justify it. Document the supported limits.
- Done when the deployment has a measured memory margin and concurrent uploads fail gracefully rather than terminating the process. Decoder process isolation is optional future work, not a prerequisite by itself.

### TD-004 — Exercise crash recovery and coordinated restore

Priority: before relying on the hosted archive. Source: documented M2 consistency limitation.

**M3 recovery evidence (partial):** `tests/recovery.rs::coordinated_m2_backup_restore_and_bootstrap` runs real `pg_dump`/`pg_restore` plus the shipped backup/restore scripts against isolated PostgreSQL 17 databases and temporary directories. It verifies restored catalog records, uploader/artist metadata, image memberships, generated storage keys/SHA-256 and exact PNG bytes, then applies M3 migrations and provisions a login. A second restore is refused. Existing orphan-maintenance tests verify dry-run/apply/grace-period behavior. [deployment.md](deployment.md) distinguishes missing files, unreferenced files, and ambiguous commits, and documents recovery. **Still open:** actual crash/power-loss experiments, directory-entry durability, and Railway-volume recovery. Ordinary restore evidence is not a claim of crash atomicity.

File publication and database commit are separate. Cancellation/crashes can leave orphans; directory entries are not separately fsynced, so power loss can also leave metadata pointing to missing files. Ambiguous commit outcomes deliberately retain files.

- Location: `src/storage.rs`, `src/app/images/mod.rs`, README maintenance/backups.
- Exercise orphan cleanup and restore of matching database/files into an isolated destination. Document recovery of missing files and consider directory syncing where supported if stronger durability is needed.
- Done when a restore is demonstrated and the operator can distinguish missing files, unreferenced files, and uncertain commits. Do not promise filesystem/database atomicity.

## Conventions to preserve

### TD-005 — Keep catalog generalization bounded

Status: respected in M2; monitor as features grow. Source: M1 review.

The shared catalog `Record`/`Kind` and multipurpose page template are manageable for the small catalog. Images correctly use dedicated models, application functions, and templates. Avoid extending the catalog with unrelated optional fields. Split catalog views/models only when a concrete change makes them difficult to maintain; no broad refactor is currently required.

## Resolved

### TD-001 — Test streamed oversized uploads

Resolved in M3 (working-tree implementation, 2026-10-04). Source: M2 review.

`tests/images.rs::streamed_multipart_limits_without_content_length` sends multi-chunk bodies with no Content-Length. It checks both a total request crossing 20 MiB + 64 KiB and a single file crossing 20 MiB within the request allowance; both return 413 with zero image rows and no permanent/temp files. A valid streamed PNG succeeds through real account login and PostgreSQL-backed upload.

This exposed nested Axum/tower body-limit errors that were previously rendered as 422. `multipart_error` now checks typed LengthLimitError sources through wrappers and returns 413. The regression and the full existing image workflow pass against PostgreSQL 17.

### TD-006 — Preserve forms for stale relationship selections

Resolved in M2 (`dd325e2`). Source: M1 review.

Previously, selecting a deleted tag/person/association type returned a generic 404. The related-record helper now returns validation errors while retaining the parent character page and submitted selection values. A stale-tag HTTP regression test was added. Future coverage can include person/type selections if this path changes.

## Verification record

- Bulk image upload (2026-10-05): formatting, Clippy, and all 35 tests passed against isolated PostgreSQL 17; the locked native release build passed. Four new integration tests cover negotiated JSON, shared/uncategorized memberships, partial validation failures, duplicates preserving attribution/metadata/memberships, lost-response retry, exact bigint IDs, ordinary HTML upload/edit compatibility, authentication/origin/maintenance, route selection, escaped options, storage failure, and native upload/gallery/edit/restart/content/deletion smoke. Streamed-body regressions now exercise both HTML and JSON negotiation and still verify 413 with no image rows or files. Ten real-script/DOM browser checks passed headlessly in the installed Firefox-compatible Zen browser using fake fetch responses. The installed Brave binary timed out even on an empty headless page, so Chromium was not verified here. Hosted bulk ingestion/proxy behavior remains owner-unverified. No deployment-tier memory measurement or stronger crash-consistency claim is added; TD-002/TD-003/TD-004 remain open as documented.
- M1 review: formatting, Clippy, and 3 tests passed against isolated PostgreSQL 17.
- M2 review: formatting, Clippy, and all 10 tests passed against isolated PostgreSQL 17.
- M3: formatting, Clippy, and 18 tests passed against isolated PostgreSQL 17, including real CLI administration, authenticated HTTP workflows, streamed-upload limits and M2 dump/restore.
- Native M3 smoke: two CLI accounts, protected catalog/shared upload/gallery/duplicate/edit/content, graceful restart with stored files and session persistence, POST logout/revocation, image deletion retaining characters, maintenance mode and orphan dry run passed. Locked native release build passed.
- Docker is unavailable here: production container/Compose startup, actual Neon/Railway HTTPS/volume behavior, and chosen deployment-tier decoder memory remain unverified. No hosted memory measurements are inferred from local tests.
- Docker Compose startup and an interactive browser smoke test were not independently verified in the earlier reviews. These are remaining verification tasks, not known defects.
- Librarian Clerk M1 (2026-10-04): formatting and Clippy passed; 31 tests passed against isolated PostgreSQL 17, including 12 new Clerk tests and the pending health regression. Coverage includes raw Ed25519 verification/timestamps, closed-pool PING/authorization/maintenance denials, website protection isolation, manifest/parser/configuration/replay bounds, exact literal/Unicode/ambiguous selectors, same-row filters/pagination, shared image memberships, unchanged canonical table counts, bounded actual file reads/fallbacks, fake multipart edits/429/timeouts/individual registration, and offline CLI independence. Native executable smoke verifies signed PING, maintenance, restart and graceful SIGTERM. Locked native release build passed. Docker remains unavailable; no real commands/messages/portal resources/deployments were created, and hosted Clerk acceptance remains owner-unverified. No deployment-tier memory or crash-recovery claim is added.

When completing an item, record the commit and verification evidence and move it to Resolved. Keep historical IDs stable.
