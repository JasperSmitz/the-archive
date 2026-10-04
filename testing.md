# Manual walkthrough: M1–M3

Use disposable test records for this walkthrough. Deletion is real; leave personal records alone. The app works without JavaScript. Accounts are access credentials; catalog people are separate domain subjects.

## Local access and controls

Open <http://127.0.0.1:3000>. For the locally started review instance, credentials are in the **ignored, mode-0600** file `var/local-login.txt`; do not commit or share it. This uses a separate `archive_local_review` database and `var/images`. Docker was unavailable, so PostgreSQL runs from `/tmp/archive-pg/bin`, bound to loopback port 55432. Its data directory is `/tmp/archive-pg-data`: it survives service restarts, but is not a durable long-term location if your machine clears `/tmp`. Back up any data you want to retain before reboot/cleanup, or move to the README's persistent Compose setup.

From the repository directory:

```sh
# Start PostgreSQL if it is stopped:
/tmp/archive-pg/bin/pg_ctl -D /tmp/archive-pg-data -l /tmp/archive-pg-server.log \
  -o '-h 127.0.0.1 -p 55432 -k /tmp' start
# Start the application using the ignored .env:
cargo run --locked
# Ctrl-C stops a foreground app. For the instance started in the background:
kill -TERM "$(cat var/app.pid)"
# Restart after stopping; use the same .env and image directory:
cargo run --locked
```

Do not start a second copy on port 3000. The current background process writes logs to `var/app.log`. Explicit exported environment variables override `.env`; unset old DATABASE_URL/APP_ENV/LISTEN_ADDR/APP_ORIGIN overrides if they point elsewhere. Never delete the PostgreSQL directory or image files to reset the app.

## 1. Private access

1. In a private/incognito window, visit `/characters`, `/images`, and `/images/1`. Expect a login redirect with no private records shown.
2. Visit `/images/1/content` while logged out. Expect 401 plain text, even if that ID does not exist. `/healthz` should show only `ok`; CSS should load without login.
3. Try a wrong password and a nonexistent username. Both should say **Invalid username or password**. The username stays populated, the password does not.
4. Sign in as `local-one`. Expect the normal catalog navigation and a Sign out button. `/login?next=%2Fimages` should return you to Images after login. `/login?next=https%3A%2F%2Fexample.com` should reject the return target.
5. Sign out. Reopen a private page and a saved original-image URL: access should be denied. Sign in again for the remaining tests.

## 2. Catalog creation and editing

Create these disposable records through the navigation:

- People: `Alice Test`, `Bob Test`.
- Franchises: `Zelda Test`, `Genshin Test`, `Moomins Test`.
- Characters: `Link` in Zelda Test, `Link` in Genshin Test, `Venti` in Genshin Test, `Snufkin` in Moomins Test. Add a multiline description to one.
- Tags: `Adventurer`, `Nature / cozy!`.
- Association type: key `travel_buddy`, label `Travel buddy`.

Expect alphabetical lists and clear empty states before adding records. A franchise is required for characters. Same-name characters in different franchises are allowed.

Edit names/description, a tag, a franchise, a person, and the new type's display label. The type's key should remain immutable. Enter `<script>alert(1)</script>` in a description: it must display as text, never execute. Multiline descriptions should keep their line breaks.

## 3. Relationships and combined filters

On character pages, add:

| Character | Person/type | Tags |
| --- | --- | --- |
| Link — Zelda Test | Alice Test / Kin; Bob Test / Favorite | Adventurer; Nature / cozy! |
| Venti — Genshin Test | Alice Test / Favorite; Alice Test / Associated | Nature / cozy! |
| Snufkin — Moomins Test | Bob Test / Associated | Nature / cozy! |

Try adding the same tag and same association twice. Expect one membership, with no duplicate error. Alice may have multiple types on Venti.

Browse Characters and verify:

- Alice alone returns Zelda Link and Venti.
- Favorite alone returns Zelda Link and Venti.
- **Alice + Favorite returns only Venti**: Link's Alice/Kin row must not combine with Bob/Favorite.
- Genshin Test + Alice + Favorite + Nature / cozy! returns only Venti. Filters combine with AND.
- A character appears once even with multiple matching associations. Selected controls survive submission; clear filters to restore the list.

Remove Alice/Associated from Venti and one tag from Link. Expect the other memberships to remain. Add and remove Travel buddy as another type.

## 4. Artwork upload, gallery, and metadata

Use a small JPEG, PNG, and static WebP of your choice. A tiny WebP fixture is available at `tests/fixtures/lossy.webp`.

1. Images → Upload artwork. Choose a file, manually select Alice Test as uploader, add artist/source URL, and select both Zelda Link and Venti. Upload should redirect to a detail page.
2. Verify image, detected format, dimensions, filename, uploader, archival time, metadata, and links to both characters. A filename's path components must never become storage paths.
3. Open Browse artwork from either associated character: the same image should appear. Open the original/content link, then try that URL in an incognito window: expect 401 there.
4. Upload the **exact same file bytes** again, with different metadata/character choices. Expect **Already archived**, the same image ID, and unchanged original metadata/memberships. Edit the existing image to add memberships instead.
5. Edit uploader to Bob Test, artist/source, and remove Link while keeping Venti. Expect the changed detail and gallery memberships; image bytes stay unchanged. Blank artist/source becomes absent metadata.
6. Upload another image with **no characters**. It should remain available in the unfiltered gallery/detail.
7. Upload JPEG/PNG/WebP with a misleading filename extension: detection should follow bytes. Galleries are newest first and use originals with lazy loading.
8. Optional pagination: upload 25 distinct files (byte-identical files deduplicate). Expect 24 on page one, a second page, and preserved character filters. `/images?page=0`, negative/non-numeric pages, and extreme overflow values should return validation errors.

## 5. Validation and conflicts

Try each using disposable values, then correct the same form:

- A whitespace-only name; a name over 200 characters; a tag over 80; description over 10,000. Expect field errors and preserved submitted values/selections.
- `alice test` as a second person, case variants of a franchise/tag, and `link` again in Zelda Test. Expect useful 409 duplicate errors. The Genshin Link is valid.
- A new type with an uppercase/spaced key or duplicate key. Expect validation/conflict. Keys use lowercase ASCII snake_case.
- Source `javascript:alert(1)`, a relative URL, or `https://user:password@example.com`. Expect 422 and preserved metadata. Valid absolute HTTP/HTTPS URLs are metadata only and are never fetched.
- Artist `<script>Artist</script>`: display escaped text. Artist over 200 characters should fail validation.
- SVG, GIF, animated PNG/WebP, corrupt/truncated files, or a file over 20 MiB. Expect rejection; an oversized file/request returns 413. Upload errors ask you to select the file again, because browsers cannot repopulate it.
- Dimensions over 12,000 or decoded pixels over 40 million are rejected. The automated tests inject small limits so huge fixtures are unnecessary.
- Missing records such as `/characters/999999999` and `/images/999999999` while signed in return 404. Stale relationship selections should produce field errors while preserving their existing parent form.

Automated tests additionally check streamed multipart requests without Content-Length, total request size versus file size, decoder limits, duplicate races, stale records, and file cleanup.

## 6. Deletion rules

Use confirmation pages and check both cancel/back and confirm:

1. Attempt to delete Alice/Bob while they have character associations or uploader attributions. Expect a useful conflict; remove/correct these dependencies first.
2. Attempt to delete Genshin Test while characters use it. Expect a conflict.
3. Attempt to delete Favorite (or the custom type) while an association uses it. Expect a conflict.
4. Delete Adventurer. Its character memberships disappear, characters remain.
5. Delete a character attached to artwork. Only its image membership disappears; the image and other linked characters remain.
6. Delete an image. Its memberships/file disappear, and its characters remain. Confirmation is required. Missing image files do not prevent removal of stale metadata; automated tests cover this without damaging your archive.

## 7. Sessions, administration, and restart persistence

Keep a signed-in window and an uploaded image, stop/restart the application with the commands above, then refresh. Expect the session, catalog, metadata, memberships, and exact original bytes to survive.

```sh
cargo run --locked -- account list
cargo run --locked -- account reset-password local-one
# Hidden prompts; reset revokes local-one's sessions in all browser windows.
cargo run --locked -- account disable local-two
# Disabling revokes sessions and blocks login. Password reset does not re-enable it.
# Use a third DISPOSABLE account instead if you want to keep local-two usable:
cargo run --locked -- account create disposable-login
cargo run --locked -- account disable disposable-login
cargo run --locked -- sessions cleanup
```

Seven-day absolute expiry is covered automatically. Login attempts are globally capped at 20 per minute: repeatedly submit invalid credentials to see 429, then wait one minute. This temporarily affects both accounts and resets on server restart.

To test maintenance, stop the app and launch `MAINTENANCE_MODE=true cargo run --locked`. Catalog/login/logout should return 503; `/healthz` stays 200. Stop it and restart normally to resume. Same-origin protection rejects unsafe requests with missing/bad Origin and Referer, including login/logout; ordinary browser forms work. Automated HTTP tests check cross-origin requests and unauthenticated mutations leave data unchanged.

## 8. Recovery and automated checks

Run orphan cleanup **dry-run only** against your active archive:

```sh
cargo run --locked -- cleanup-orphans
```

Use `--apply` only with all uploads stopped and after inspecting candidates. Only generated orphan/temp files older than 24 hours are eligible; referenced files and unrelated files are retained.

Follow deployment.md for coordinated backups and restore into a **new empty database and empty image directory**. Do not restore over this running archive. Database/filesystem operations are not crash-atomic. The recovery test actually dumps/restores an isolated M2 fixture, checks matching metadata/memberships/bytes, applies M3, and provisions a login.

For the current local PostgreSQL installation:

```sh
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
PG_BIN=/tmp/archive-pg/bin DATABASE_URL=postgres://archive@127.0.0.1:55432/postgres cargo test
```

Tests create isolated databases and temporary image directories, never wipe your review catalog. Docker/container, actual Railway/Neon HTTPS/volume behavior, and deployment-tier memory still require provider-side verification. No deployment is performed by this walkthrough.
