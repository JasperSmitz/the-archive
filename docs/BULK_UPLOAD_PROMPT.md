# Archive milestone: bulk image upload

Implement a focused website bulk-upload workflow. Inspect the repository, applicable instructions, image upload/domain/storage code, authentication, tests, and deployment documentation before editing. Follow existing Rust/Axum/Askama/SQLx conventions. Do not redesign the application.

## Goal

An owner can select a collection of local artwork files, select uploader attribution and one or more characters once, and archive the collection with one action. For example: select 30 images, choose Zelda and Link, then upload. Each newly archived image receives both character memberships. Make clear that selecting both means every image depicts both; mixed collections should be uploaded as separate batches or corrected individually afterward.

## Architecture and scope

- Add a discoverable bulk-upload page linked from the gallery and ordinary upload page. Keep the existing single-image upload and metadata editing functional, including without JavaScript.
- Use a small ordinary JavaScript file, served by the existing application, for the batch workflow. No frontend framework, build pipeline, queue, background worker, or new infrastructure.
- Send one file per multipart request, sequentially. Reuse the existing protected upload route with explicit JSON response negotiation, or a small protected adapter over the same application upload function. Keep redirects/rendered errors for existing HTML clients. Do not duplicate validation, persistence, SHA-256 handling, or filesystem cleanup logic.
- Preserve the existing per-request/per-file body limits and decoder concurrency limits. Never buffer a complete batch on the server. Do not increase limits to accommodate one giant multipart body.
- No schema migration or persistent batch/job record is expected. No Discord changes, remote URL fetching, archive/ZIP import, folder scanning, AI, thumbnail pipeline, or image processing beyond existing validation.

## User experience

- Multiple-file picker accepting existing supported formats, with a maximum of 100 files per selection. Explain the limit and existing file/format/dimension limits. Apply the count limit in the UI; the server continues to accept only one bounded file per request.
- Shared uploader attribution and optional character memberships, matching existing semantics. Zero characters remains allowed. Snapshot the shared values when the run starts; disable editing them during the run.
- Optional shared artist/source fields may reuse existing controls, but label clearly that they apply to every new image. Explain that different artists/sources require separate batches or individual edits. Do not infer artist/source from filenames or invent provenance.
- List selected filenames and sizes, permit removal before starting, show overall completed/total counts and per-file pending/uploading/uploaded/duplicate/failed/unconfirmed outcomes. Escape all filenames and messages; use safe DOM text APIs.
- Show per-file links to uploaded or existing duplicate records and an accurate final summary. A duplicate is a distinct successful outcome, not a failure or a new image.
- Continue after an ordinary per-file validation failure. Retain File objects and shared values while the page is open so users do not have to reselect failed files. Offer an explicit retry of failed/unconfirmed files, skipping already uploaded/duplicate files. No automatic network retries.
- Stop starting further requests on session expiry, maintenance, or a network/server failure indicating availability is uncertain. Give an actionable message and retain pending files for an explicit resume. A response lost after submission is unconfirmed, not proof the file was not archived; explain that retrying safely resolves through exact duplicate detection.
- Prevent double submission/concurrent runs in the same page. A Stop control should stop scheduling additional files and allow the in-flight request to settle. Navigating away may interrupt the run; completed images remain saved. Do not promise background persistence or cross-reload resume. Explain this before starting.
- JavaScript-disabled users see a useful link to the ordinary upload form.

## Domain and duplicate behavior

- Each file commits independently through existing image upload logic; a later failure never rolls back earlier successes. Do not implement batch-wide transactions across requests/files.
- Maintain static JPEG/PNG/WebP validation, 20 MiB file limit, existing axis/pixel/decoder limits, generated safe storage keys, exact SHA-256 deduplication, and current cleanup behavior including uncertain commit outcomes.
- Existing duplicates, including repeated files within this batch, return the existing image ID. Do not overwrite uploader/artist/source or add/remove character memberships on duplicate images. State this clearly in the result UI; link to the existing metadata editor if the owner wants to change assignments.
- Validate shared metadata server-side on every request. Browser checks are convenience only. Never trust MIME declarations, filenames, selected IDs, or client outcome claims.

## HTTP/security conventions

- Every batch request must pass existing website session, same-origin, and maintenance protections. Do not add an anonymous route or bypass middleware. Original image URLs remain protected.
- Return bounded structured JSON for batch requests with uploaded/duplicate ID or a safe error message and useful HTTP status. Keep internal database/storage details out of browser responses. Distinguish per-file validation from authentication, maintenance, and unexpected server failures using actual status/response behavior.
- Explicitly detect redirected login/HTML responses rather than reporting them as successful JSON uploads. Handle missing/expired sessions and body-limit failures gracefully, including failures emitted by middleware before the handler.
- Preserve existing cache and logging policies. Do not log image bytes, credentials, or entire multipart bodies. Shared metadata uses existing normalization/validation.

## Acceptance and checks

1. Selecting several valid files and two characters creates one record per distinct file, each with both memberships and shared attribution. Existing single upload and editing still work.
2. A mixed batch of valid, corrupt, unsupported, oversized, and duplicate files reports accurate individual outcomes; valid files persist despite validation failures. Tests should cover real streaming/body limits, not only browser size checks.
3. Duplicate files within the batch and already in the archive return existing IDs without changing existing metadata/memberships. Retrying after a lost response does not create duplicate records.
4. Server rejects unauthenticated, wrong-origin, invalid-metadata, and maintenance requests through the ordinary protections. Expiry/unavailability stops scheduling later requests; prior successes remain visible.
5. UI makes one request at a time, freezes batch metadata during the run, prevents double start, supports stop/resume and explicit failed/unconfirmed retry, and displays filenames safely. Use the project's existing testing approach; add meaningful server integration tests and a practical browser/manual checklist. Do not introduce a browser-test framework solely for this milestone.
6. Review bounded memory/request behavior. Do not add a batch-sized server allocation or parallel decoder load. Verify routes do not interfere with existing `/images/{id}` routing.
7. Update README/relevant image documentation and roadmap with workflow, duplicate policy, limits, partial completion, interruption/retry behavior, and any operator changes. No new environment variables or deployment resources are expected.
8. Run formatting, Clippy, and relevant tests against an isolated local test database; broaden to the existing suite if HTTP/domain behavior changes. Do not test against production or real Discord. Report checks run and any unavailable checks honestly.

Deliver the implementation, explain changes and limitations, and report architectural questions or follow-ups. Do not deploy or push without owner authorization. Keep this milestone focused on making manual Archive ingestion pleasant.
